//! Forge and NeoForge support.
//!
//! Unlike Fabric (whose meta server hands us a complete library list we can
//! download directly), Forge and NeoForge client installs require running the
//! official installer: it binary-patches the vanilla client and generates
//! libraries that exist on no maven. We run that installer headlessly
//! (`java -jar installer.jar --installClient <root>`) against our data dir,
//! which already uses the standard versions/ + libraries/ layout, then merge
//! the partial version JSON it produces with the vanilla one so the normal
//! install and launch pipeline can take over.

use crate::download::{self, CancelFlag, DownloadJob, VerifyMode};
use crate::java::{self, JavaInstall};
use crate::manifest::VersionManifest;
use crate::paths::{ensure_parent, Paths};
use crate::version::{self, Arguments, Library, VersionDetails};
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeKind {
    Forge,
    NeoForge,
}

impl ForgeKind {
    pub fn label(self) -> &'static str {
        match self {
            ForgeKind::Forge => "Forge",
            ForgeKind::NeoForge => "NeoForge",
        }
    }
}

const FORGE_PROMOS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const NEOFORGE_META: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases/net/neoforged/neoforge";

/// Map of MC version -> Forge build to install (recommended where one exists,
/// otherwise latest). Limited to MC 1.13+, where the installer supports the
/// headless `--installClient` mode we depend on.
pub async fn fetch_forge_versions(client: &reqwest::Client) -> Result<HashMap<String, String>> {
    #[derive(Deserialize)]
    struct Promos {
        promos: HashMap<String, String>,
    }
    let p: Promos = client
        .get(FORGE_PROMOS)
        .send()
        .await
        .context("fetching Forge promotions")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Forge promotions")?;
    let mut recommended = HashMap::new();
    let mut latest = HashMap::new();
    for (key, ver) in p.promos {
        let Some((mc, channel)) = key.rsplit_once('-') else {
            continue;
        };
        if !headless_installer_supported(mc) {
            continue;
        }
        match channel {
            "recommended" => {
                recommended.insert(mc.to_string(), ver);
            }
            "latest" => {
                latest.insert(mc.to_string(), ver);
            }
            _ => {}
        }
    }
    for (mc, v) in latest {
        recommended.entry(mc).or_insert(v);
    }
    Ok(recommended)
}

fn headless_installer_supported(mc: &str) -> bool {
    minor_of(mc).map(|m| m >= 13).unwrap_or(false)
}

fn minor_of(mc: &str) -> Option<u32> {
    let mut it = mc.split('.');
    if it.next()? != "1" {
        return None;
    }
    it.next()?.parse().ok()
}

/// Map of MC version -> newest NeoForge build, preferring stable builds over
/// betas. Covers MC 1.20.2+ (earlier NeoForge reused Forge's coordinates).
pub async fn fetch_neoforge_versions(client: &reqwest::Client) -> Result<HashMap<String, String>> {
    let xml = client
        .get(NEOFORGE_META)
        .send()
        .await
        .context("fetching NeoForge version list")?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_neoforge_metadata(&xml))
}

/// NeoForge versions read `<mc minor>.<mc patch>.<build>[-beta]`; the maven
/// metadata lists oldest-first, so the last entry per MC version is newest.
fn parse_neoforge_metadata(xml: &str) -> HashMap<String, String> {
    let mut stable: HashMap<String, String> = HashMap::new();
    let mut any: HashMap<String, String> = HashMap::new();
    for chunk in xml.split("<version>").skip(1) {
        let Some(ver) = chunk.split("</version>").next() else {
            continue;
        };
        let ver = ver.trim();
        let Some(mc) = neoforge_to_mc(ver) else {
            continue;
        };
        any.insert(mc.clone(), ver.to_string());
        if !ver.contains('-') {
            stable.insert(mc, ver.to_string());
        }
    }
    for (mc, v) in any {
        stable.entry(mc).or_insert(v);
    }
    stable
}

fn neoforge_to_mc(ver: &str) -> Option<String> {
    let base = ver.split('-').next().unwrap_or(ver);
    let parts: Vec<&str> = base.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let minor: u32 = parts[0].parse().ok()?;
    let patch: u32 = parts[1].parse().ok()?;
    parts[2].parse::<u32>().ok()?;
    // The 1.20.1-era releases reused Forge's 47.x numbering; they don't
    // follow the MC-derived scheme, so exclude them.
    if !(20..40).contains(&minor) {
        return None;
    }
    Some(if patch == 0 {
        format!("1.{minor}")
    } else {
        format!("1.{minor}.{patch}")
    })
}

/// The version id the installer creates, which we also use as our instance id.
pub fn version_id(kind: ForgeKind, mc: &str, loader_version: &str) -> String {
    match kind {
        ForgeKind::Forge => format!("{mc}-forge-{loader_version}"),
        ForgeKind::NeoForge => format!("neoforge-{loader_version}"),
    }
}

fn installer_url(kind: ForgeKind, mc: &str, loader_version: &str) -> String {
    match kind {
        ForgeKind::Forge => format!(
            "{FORGE_MAVEN}/{mc}-{lv}/forge-{mc}-{lv}-installer.jar",
            lv = loader_version
        ),
        ForgeKind::NeoForge => format!(
            "{NEOFORGE_MAVEN}/{lv}/neoforge-{lv}-installer.jar",
            lv = loader_version
        ),
    }
}

/// Prepare the merged version JSON for a Forge/NeoForge install, running the
/// official installer if this loader+MC combination isn't on disk yet.
/// After this returns, the existing install pipeline can take over from the
/// JSON at `paths.version_json(&returned_id)`.
///
/// `out_id` writes an additional copy of the merged JSON under a custom id
/// (used by the modpack installer); `None` keeps the natural id.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_version(
    client: &reqwest::Client,
    paths: &Paths,
    manifest: &VersionManifest,
    kind: ForgeKind,
    mc_version: &str,
    loader_version: &str,
    out_id: Option<&str>,
    cancel: &CancelFlag,
    progress: impl Fn(String),
) -> Result<String> {
    let natural_id = version_id(kind, mc_version, loader_version);
    if load_complete_details(paths, &natural_id).is_none() {
        run_installer_and_merge(
            client,
            paths,
            manifest,
            kind,
            mc_version,
            loader_version,
            &natural_id,
            cancel,
            &progress,
        )
        .await?;
    }
    let Some(merged) = load_complete_details(paths, &natural_id) else {
        bail!("{} install produced no usable version JSON", kind.label());
    };
    if let Some(out) = out_id {
        if out != natural_id {
            let mut clone = merged;
            clone.id = out.to_string();
            let p = paths.version_json(out);
            ensure_parent(&p)?;
            tokio::fs::write(&p, serde_json::to_vec_pretty(&clone)?).await?;
            return Ok(out.to_string());
        }
    }
    Ok(natural_id)
}

/// A version JSON counts as installed only if it parses as the FULL merged
/// schema. The installer's own output is a partial (no assets/downloads), so
/// it fails this parse and triggers the merge.
fn load_complete_details(paths: &Paths, id: &str) -> Option<VersionDetails> {
    let p = paths.version_json(id);
    let bytes = std::fs::read(p).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[allow(clippy::too_many_arguments)]
async fn run_installer_and_merge(
    client: &reqwest::Client,
    paths: &Paths,
    manifest: &VersionManifest,
    kind: ForgeKind,
    mc_version: &str,
    loader_version: &str,
    natural_id: &str,
    cancel: &CancelFlag,
    progress: &impl Fn(String),
) -> Result<()> {
    let entry = manifest
        .versions
        .iter()
        .find(|v| v.id == mc_version)
        .ok_or_else(|| anyhow!("Minecraft version '{mc_version}' not found in manifest"))?;
    let vanilla = ensure_vanilla(client, paths, mc_version, &entry.url, cancel, progress).await?;

    progress(format!(
        "Downloading {} {loader_version} installer",
        kind.label()
    ));
    let installer = download_installer(client, paths, kind, mc_version, loader_version, cancel).await?;

    run_installer(paths, &vanilla, kind, &installer, cancel, progress).await?;

    let partial_path = locate_installed_json(paths, natural_id, loader_version)
        .ok_or_else(|| anyhow!("{} installer finished but produced no version JSON", kind.label()))?;
    let bytes = tokio::fs::read(&partial_path).await?;
    let profile: LoaderProfile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {}", partial_path.display()))?;

    let merged = merge_profile(&vanilla, profile, natural_id);
    let out = paths.version_json(natural_id);
    ensure_parent(&out)?;
    tokio::fs::write(&out, serde_json::to_vec_pretty(&merged)?).await?;
    Ok(())
}

/// The installer needs the vanilla version JSON + client jar in place.
async fn ensure_vanilla(
    client: &reqwest::Client,
    paths: &Paths,
    mc_version: &str,
    url: &str,
    cancel: &CancelFlag,
    progress: &impl Fn(String),
) -> Result<VersionDetails> {
    let vj = paths.version_json(mc_version);
    let details: VersionDetails = match tokio::fs::read(&vj).await {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing cached {}", vj.display()))?,
        Err(_) => {
            let d = version::fetch_details(client, url).await?;
            ensure_parent(&vj)?;
            tokio::fs::write(&vj, serde_json::to_vec_pretty(&d)?).await?;
            d
        }
    };
    progress(format!("Fetching Minecraft {mc_version} client jar"));
    let job = DownloadJob {
        url: details.downloads.client.url.clone(),
        dest: paths.version_jar(mc_version),
        sha1: Some(details.downloads.client.sha1.clone()),
        size: details.downloads.client.size,
    };
    // Progress events are dropped — the closed receiver makes sends no-ops.
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    download::run_jobs(client, std::slice::from_ref(&job), &tx, VerifyMode::Fast, cancel).await?;
    Ok(details)
}

async fn download_installer(
    client: &reqwest::Client,
    paths: &Paths,
    kind: ForgeKind,
    mc_version: &str,
    loader_version: &str,
    cancel: &CancelFlag,
) -> Result<PathBuf> {
    let url = installer_url(kind, mc_version, loader_version);
    let name = url.rsplit('/').next().unwrap_or("installer.jar");
    let dest = paths.cache.join("installers").join(name);
    let job = DownloadJob {
        url,
        dest: dest.clone(),
        sha1: None,
        size: 0,
    };
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    download::run_jobs(client, std::slice::from_ref(&job), &tx, VerifyMode::Fast, cancel).await?;
    Ok(dest)
}

async fn run_installer(
    paths: &Paths,
    vanilla: &VersionDetails,
    kind: ForgeKind,
    installer: &std::path::Path,
    cancel: &CancelFlag,
    progress: &impl Fn(String),
) -> Result<()> {
    // The installer refuses to run unless the target looks like a launcher dir.
    let lp = paths.root.join("launcher_profiles.json");
    if !lp.exists() {
        std::fs::write(&lp, b"{\"profiles\":{}}")?;
    }

    let required = vanilla
        .java_version
        .as_ref()
        .map(|j| j.major_version)
        .unwrap_or(8);
    let java = pick_java(required, kind)?;
    progress(format!(
        "Running {} installer with Java {}...",
        kind.label(),
        java.major
    ));

    let mut cmd = tokio::process::Command::new(&java.path);
    cmd.arg("-jar")
        .arg(installer)
        .arg("--installClient")
        .arg(&paths.root);
    cmd.current_dir(&paths.root);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning the {} installer", kind.label()))?;

    let mut out_lines = BufReader::new(child.stdout.take().expect("stdout piped")).lines();
    let mut err_lines = BufReader::new(child.stderr.take().expect("stderr piped")).lines();
    let mut out_done = false;
    let mut err_done = false;
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let handle = |line: String, tail: &mut std::collections::VecDeque<String>| {
        let line = line.trim().to_string();
        if line.is_empty() {
            return;
        }
        if tail.len() >= 12 {
            tail.pop_front();
        }
        tail.push_back(line.clone());
        progress(format!("{}: {line}", kind.label()));
    };
    while !(out_done && err_done) {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            bail!("cancelled by user");
        }
        tokio::select! {
            line = out_lines.next_line(), if !out_done => match line {
                Ok(Some(l)) => handle(l, &mut tail),
                _ => out_done = true,
            },
            line = err_lines.next_line(), if !err_done => match line {
                Ok(Some(l)) => handle(l, &mut tail),
                _ => err_done = true,
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
        }
    }
    let status = child.wait().await?;
    if !status.success() {
        bail!(
            "{} installer failed (exit {}). Last output: {}",
            kind.label(),
            status.code().unwrap_or(-1),
            tail.iter().cloned().collect::<Vec<_>>().join(" | ")
        );
    }
    Ok(())
}

fn pick_java(required: u32, kind: ForgeKind) -> Result<JavaInstall> {
    if let Some(j) = java::detect_for_major(required) {
        return Ok(j);
    }
    let mut candidates: Vec<JavaInstall> = java::detect_all()
        .into_iter()
        .filter(|j| j.major >= required)
        .collect();
    candidates.sort_by_key(|j| j.major);
    candidates.into_iter().next().ok_or_else(|| {
        anyhow!(
            "the {} installer needs Java {required}+, but none was found. Install Java {required} and try again.",
            kind.label()
        )
    })
}

/// Find the version JSON the installer wrote. Normally it lands exactly at
/// `versions/<natural_id>/`, but scan for a near-miss in case the id format
/// shifts between loader releases.
fn locate_installed_json(paths: &Paths, natural_id: &str, loader_version: &str) -> Option<PathBuf> {
    let direct = paths.version_json(natural_id);
    if direct.exists() {
        return Some(direct);
    }
    let rd = std::fs::read_dir(&paths.versions).ok()?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.contains(loader_version) && name.to_ascii_lowercase().contains("forge") {
            let p = e.path().join(format!("{name}.json"));
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// The partial version JSON the installer writes: inheritsFrom + the loader's
/// own mainClass, libraries, and extra arguments.
#[derive(Debug, Deserialize)]
struct LoaderProfile {
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(default)]
    arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments", default)]
    minecraft_arguments: Option<String>,
}

/// Flatten the installer's partial JSON onto the vanilla parent, mirroring
/// the official launcher's inheritsFrom semantics: loader libraries first on
/// the classpath, loader arguments appended after the parent's.
fn merge_profile(vanilla: &VersionDetails, profile: LoaderProfile, id: &str) -> VersionDetails {
    let mut merged = vanilla.clone();
    merged.id = id.to_string();
    merged.main_class = profile.main_class;
    let mut libs = profile.libraries;
    libs.extend(vanilla.libraries.clone());
    merged.libraries = libs;
    if let Some(pa) = profile.arguments {
        let mut args = merged.arguments.unwrap_or(Arguments {
            game: Vec::new(),
            jvm: Vec::new(),
        });
        args.game.extend(pa.game);
        args.jvm.extend(pa.jvm);
        merged.arguments = Some(args);
    }
    if let Some(legacy) = profile.minecraft_arguments {
        merged.minecraft_arguments = Some(legacy);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_versions_map_to_mc() {
        assert_eq!(neoforge_to_mc("20.4.237"), Some("1.20.4".to_string()));
        assert_eq!(neoforge_to_mc("21.0.167"), Some("1.21".to_string()));
        assert_eq!(neoforge_to_mc("21.1.77"), Some("1.21.1".to_string()));
        assert_eq!(neoforge_to_mc("21.4.10-beta"), Some("1.21.4".to_string()));
        // Legacy 1.20.1-era coordinates reused Forge's 47.x numbering; skipped.
        assert_eq!(neoforge_to_mc("47.1.84"), None);
    }

    #[test]
    fn neoforge_metadata_prefers_stable_per_mc() {
        let xml = r#"
            <versions>
              <version>20.4.100</version>
              <version>20.4.237</version>
              <version>21.4.1-beta</version>
              <version>21.4.2-beta</version>
            </versions>"#;
        let map = parse_neoforge_metadata(xml);
        assert_eq!(map.get("1.20.4"), Some(&"20.4.237".to_string()));
        // No stable exists for 1.21.4 yet, so the newest beta wins.
        assert_eq!(map.get("1.21.4"), Some(&"21.4.2-beta".to_string()));
    }

    #[test]
    fn forge_headless_gate_is_1_13_plus() {
        assert!(headless_installer_supported("1.13.2"));
        assert!(headless_installer_supported("1.20.1"));
        assert!(!headless_installer_supported("1.12.2"));
        assert!(!headless_installer_supported("1.7.10"));
    }

    #[test]
    fn version_ids() {
        assert_eq!(
            version_id(ForgeKind::Forge, "1.20.1", "47.3.0"),
            "1.20.1-forge-47.3.0"
        );
        assert_eq!(
            version_id(ForgeKind::NeoForge, "1.21.1", "21.1.77"),
            "neoforge-21.1.77"
        );
    }

    // End-to-end installer runs: network + a local JDK required, so these are
    // ignored by default. Run with `cargo test -- --ignored` to exercise the
    // full pipeline against the live Forge/NeoForge mavens.

    fn e2e_paths(tag: &str) -> Paths {
        let root = std::env::temp_dir().join(format!("tinux-e2e-{tag}"));
        let assets = root.join("assets");
        let p = Paths {
            versions: root.join("versions"),
            libraries: root.join("libraries"),
            assets_indexes: assets.join("indexes"),
            assets_objects: assets.join("objects"),
            assets,
            natives: root.join("natives"),
            instances: root.join("instances"),
            runtimes: root.join("runtimes"),
            vanilla_minecraft: root.join(".minecraft"),
            logs: root.join("logs"),
            cache: root.join("cache"),
            root,
        };
        for d in [
            &p.root,
            &p.versions,
            &p.libraries,
            &p.assets_indexes,
            &p.assets_objects,
            &p.natives,
            &p.instances,
            &p.cache,
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        p
    }

    #[tokio::test]
    #[ignore = "network + JDK: runs the real NeoForge installer and full install pass"]
    async fn e2e_neoforge_install() {
        let client = reqwest::Client::builder()
            .user_agent("tinux-launcher-e2e")
            .build()
            .unwrap();
        let paths = e2e_paths("neoforge");
        let manifest = crate::manifest::fetch(&client).await.unwrap();
        let neo = fetch_neoforge_versions(&client).await.unwrap();
        let (mc, lv) = neo
            .iter()
            .filter(|(_, v)| !v.contains('-'))
            .max_by_key(|(mc, _)| mc.to_string())
            .map(|(mc, v)| (mc.clone(), v.clone()))
            .expect("no stable NeoForge versions");
        eprintln!("e2e: NeoForge {lv} for MC {mc}");

        let cancel = download::new_cancel_flag();
        let id = prepare_version(
            &client,
            &paths,
            &manifest,
            ForgeKind::NeoForge,
            &mc,
            &lv,
            None,
            &cancel,
            |what| eprintln!("  {what}"),
        )
        .await
        .unwrap();
        assert_eq!(id, version_id(ForgeKind::NeoForge, &mc, &lv));

        // Merged JSON must parse as the full schema with NeoForge's entrypoint.
        let merged = load_complete_details(&paths, &id).expect("merged JSON incomplete");
        assert_ne!(merged.main_class, "net.minecraft.client.main.Main");
        assert!(merged.libraries.iter().any(|l| l.name.contains("neoforge")));

        // The whole install pipeline must succeed over it, including the
        // locally-generated no-URL libraries.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        download::install_version(&client, &paths, &id, "", &tx, VerifyMode::Fast, &cancel)
            .await
            .expect("install pass over merged NeoForge version failed");
        eprintln!("e2e: NeoForge {id} installed OK");
    }

    #[tokio::test]
    #[ignore = "network + JDK: runs the real Forge installer"]
    async fn e2e_forge_install() {
        let client = reqwest::Client::builder()
            .user_agent("tinux-launcher-e2e")
            .build()
            .unwrap();
        let paths = e2e_paths("forge");
        let manifest = crate::manifest::fetch(&client).await.unwrap();
        let forge = fetch_forge_versions(&client).await.unwrap();
        let mc = "1.20.1";
        let lv = forge.get(mc).expect("no Forge build for 1.20.1").clone();
        eprintln!("e2e: Forge {lv} for MC {mc}");

        let cancel = download::new_cancel_flag();
        let id = prepare_version(
            &client,
            &paths,
            &manifest,
            ForgeKind::Forge,
            mc,
            &lv,
            None,
            &cancel,
            |what| eprintln!("  {what}"),
        )
        .await
        .unwrap();
        assert_eq!(id, version_id(ForgeKind::Forge, mc, &lv));

        let merged = load_complete_details(&paths, &id).expect("merged JSON incomplete");
        assert_eq!(merged.main_class, "cpw.mods.bootstraplauncher.BootstrapLauncher");
        assert!(merged
            .libraries
            .iter()
            .any(|l| l.name.contains("minecraftforge")));
        eprintln!("e2e: Forge {id} prepared OK");
    }
}
