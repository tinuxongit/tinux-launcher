use crate::manifest::VersionManifest;
use crate::paths::{ensure_parent, Paths};
use crate::version::{self, ArgEntry, Arguments, Artifact, Library, LibraryDownloads};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

const META_BASE: &str = "https://meta.fabricmc.net/v2";

#[derive(Debug, Clone, Deserialize)]
pub struct LoaderEntry {
    pub version: String,
    #[serde(default)]
    pub stable: bool,
}

#[derive(Debug, Deserialize)]
struct FabricProfile {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(default)]
    libraries: Vec<FabricLibrary>,
    #[serde(default)]
    arguments: Option<FabricArgs>,
}

#[derive(Debug, Deserialize)]
struct FabricLibrary {
    name: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FabricArgs {
    #[serde(default)]
    game: Vec<serde_json::Value>,
    #[serde(default)]
    jvm: Vec<serde_json::Value>,
}

pub async fn fetch_loaders(client: &reqwest::Client) -> Result<Vec<LoaderEntry>> {
    let url = format!("{META_BASE}/versions/loader");
    let entries: Vec<LoaderEntry> = client
        .get(&url)
        .send()
        .await
        .context("fetching Fabric loader list")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Fabric loader list")?;
    Ok(entries)
}

pub async fn fetch_supported_mc_versions(client: &reqwest::Client) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct GameVersion {
        version: String,
    }
    // Return EVERY version Fabric publishes an intermediary for — stable +
    // snapshots + pre-releases. The Versions tab then re-filters by kind via
    // the Snapshots/Older toggles. Returning only stable here meant turning
    // Snapshots on in the Modded tab silently produced an empty list.
    let url = format!("{META_BASE}/versions/game");
    let entries: Vec<GameVersion> = client
        .get(&url)
        .send()
        .await
        .context("fetching Fabric supported game versions")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Fabric game versions")?;
    Ok(entries.into_iter().map(|g| g.version).collect())
}

/// Prepares the merged version JSON for a Fabric profile on disk.
/// After this returns, the existing install pipeline can install the result
/// by reading the JSON at `paths.version_json(&fabric_id)`.
///
/// Returns the synthetic version id, e.g. `fabric-loader-0.15.0-1.20.4`.
pub async fn prepare_fabric_version(
    client: &reqwest::Client,
    paths: &Paths,
    manifest: &VersionManifest,
    mc_version: &str,
    loader_version: &str,
) -> Result<String> {
    let vanilla = manifest
        .versions
        .iter()
        .find(|v| v.id == mc_version)
        .ok_or_else(|| anyhow!("Minecraft version '{mc_version}' not found in manifest"))?;

    let vanilla_details = version::fetch_details(client, &vanilla.url)
        .await
        .with_context(|| format!("fetching vanilla {mc_version} version JSON"))?;

    let profile_url =
        format!("{META_BASE}/versions/loader/{mc_version}/{loader_version}/profile/json");
    let profile: FabricProfile = client
        .get(&profile_url)
        .send()
        .await
        .context("fetching Fabric profile JSON")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Fabric profile JSON")?;

    let mut merged = vanilla_details.clone();
    merged.id = profile.id.clone();
    merged.main_class = profile.main_class.clone();

    let mut new_libs: Vec<Library> = profile
        .libraries
        .into_iter()
        .filter_map(convert_library)
        .collect();
    new_libs.extend(vanilla_details.libraries);
    merged.libraries = new_libs;

    if let Some(fa) = profile.arguments {
        let game_extra = fabric_args_to_entries(&fa.game);
        let jvm_extra = fabric_args_to_entries(&fa.jvm);
        if !game_extra.is_empty() || !jvm_extra.is_empty() {
            let mut args = merged.arguments.unwrap_or(Arguments {
                game: Vec::new(),
                jvm: Vec::new(),
            });
            // Fabric jvm flags must come BEFORE the main class is set,
            // so prepend them so any vanilla flags still apply afterward.
            let mut jvm = jvm_extra;
            jvm.extend(args.jvm);
            args.jvm = jvm;
            args.game.extend(game_extra);
            merged.arguments = Some(args);
        }
    }

    let out_path = paths.version_json(&profile.id);
    ensure_parent(&out_path)?;
    let bytes = serde_json::to_vec_pretty(&merged).context("serializing merged Fabric JSON")?;
    tokio::fs::write(&out_path, bytes)
        .await
        .with_context(|| format!("writing {}", out_path.display()))?;

    Ok(profile.id)
}

fn convert_library(lib: FabricLibrary) -> Option<Library> {
    let (group, artifact, ver) = parse_maven(&lib.name)?;
    let path = format!(
        "{}/{}/{}/{}-{}.jar",
        group.replace('.', "/"),
        artifact,
        ver,
        artifact,
        ver
    );
    let base = lib
        .url
        .unwrap_or_else(|| "https://maven.fabricmc.net/".to_string());
    let base = if base.ends_with('/') { base } else { format!("{base}/") };
    let url = format!("{base}{path}");
    Some(Library {
        name: lib.name,
        downloads: Some(LibraryDownloads {
            artifact: Some(Artifact {
                path,
                url,
                sha1: lib.sha1.unwrap_or_default(),
                size: lib.size.unwrap_or(0),
            }),
            classifiers: None,
        }),
        rules: None,
        natives: None,
        extract: None,
    })
}

fn parse_maven(coord: &str) -> Option<(String, String, String)> {
    // Strip any classifier/extension suffix.
    let main = coord.split('@').next()?;
    let parts: Vec<&str> = main.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
}

fn fabric_args_to_entries(values: &[serde_json::Value]) -> Vec<ArgEntry> {
    values
        .iter()
        .filter_map(|v| v.as_str().map(|s| ArgEntry::Simple(s.to_string())))
        .collect()
}
