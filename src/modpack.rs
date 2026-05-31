//! Modrinth modpack (`.mrpack`) parsing and installation.
//!
//! A `.mrpack` is a ZIP holding `modrinth.index.json` (the file list +
//! dependencies) plus optional `overrides/` / `client-overrides/` trees that
//! are copied verbatim into the instance. We install a modpack as its own
//! launchable instance — see `worker::do_install_modpack`.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ModpackIndex {
    #[serde(default, rename = "formatVersion")]
    pub format_version: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "versionId")]
    pub version_id: String,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    #[serde(default)]
    pub files: Vec<IndexFile>,
}

#[derive(Debug, Deserialize)]
pub struct IndexFile {
    pub path: String,
    #[serde(default)]
    pub hashes: FileHashes,
    #[serde(default)]
    pub env: Option<FileEnv>,
    #[serde(default)]
    pub downloads: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct FileHashes {
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct FileEnv {
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub server: Option<String>,
}

/// What runtime the modpack targets. Tinux is Fabric-only.
pub struct LoaderReq {
    pub mc_version: String,
    pub loader_version: String,
}

/// Read `modrinth.index.json` out of a `.mrpack` archive.
pub fn parse_mrpack(bytes: &[u8]) -> Result<ModpackIndex> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("opening .mrpack archive")?;
    let mut entry = zip
        .by_name("modrinth.index.json")
        .context("modrinth.index.json not found in .mrpack")?;
    let mut s = String::new();
    entry
        .read_to_string(&mut s)
        .context("reading modrinth.index.json")?;
    serde_json::from_str(&s).context("parsing modrinth.index.json")
}

/// Validate the modpack targets Fabric and pull out the Minecraft version.
/// Forge / NeoForge / Quilt packs are rejected with a clear message.
pub fn loader_requirement(index: &ModpackIndex) -> Result<LoaderReq> {
    let mc = index
        .dependencies
        .get("minecraft")
        .cloned()
        .ok_or_else(|| anyhow!("modpack index is missing its Minecraft version"))?;
    if let Some(v) = index.dependencies.get("forge") {
        bail!("this modpack needs Forge {v}; Tinux only supports Fabric modpacks");
    }
    if let Some(v) = index.dependencies.get("neoforge") {
        bail!("this modpack needs NeoForge {v}; Tinux only supports Fabric modpacks");
    }
    if let Some(v) = index.dependencies.get("quilt-loader") {
        bail!("this modpack needs Quilt {v}; Tinux only supports Fabric modpacks");
    }
    let loader = index
        .dependencies
        .get("fabric-loader")
        .cloned()
        .ok_or_else(|| anyhow!("this modpack doesn't target Fabric; Tinux only supports Fabric modpacks"))?;
    Ok(LoaderReq {
        mc_version: mc,
        loader_version: loader,
    })
}

/// Download every client-relevant file in the index into `instance_dir` at its
/// declared path, SHA1-verifying each. `progress(done, total, what)` is called
/// before each download.
pub async fn install_files(
    client: &reqwest::Client,
    index: &ModpackIndex,
    instance_dir: &Path,
    progress: impl Fn(u64, u64, String),
) -> Result<usize> {
    let wanted: Vec<&IndexFile> = index.files.iter().filter(|f| client_wanted(f)).collect();
    let total = wanted.len() as u64;
    let mut done = 0u64;
    for f in wanted {
        let dest = safe_join(instance_dir, &f.path)?;
        let url = f
            .downloads
            .first()
            .ok_or_else(|| anyhow!("modpack file {} has no download URL", f.path))?;
        progress(done, total, format!("Downloading {}", short_name(&f.path)));
        download_verified(client, url, &dest, f.hashes.sha1.as_deref())
            .await
            .with_context(|| format!("downloading {}", f.path))?;
        done += 1;
    }
    Ok(done as usize)
}

/// Copy the `overrides/` and `client-overrides/` trees onto the instance root.
/// Synchronous (ZIP IO) — call from a blocking context.
pub fn extract_overrides(bytes: &[u8], instance_dir: &Path) -> Result<usize> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).context("opening .mrpack for overrides")?;
    let mut written = 0usize;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        let rel = match name
            .strip_prefix("overrides/")
            .or_else(|| name.strip_prefix("client-overrides/"))
        {
            Some(r) => r,
            None => continue,
        };
        if rel.is_empty() || name.ends_with('/') {
            continue;
        }
        let dest = safe_join(instance_dir, rel)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        std::fs::write(&dest, &buf)?;
        written += 1;
    }
    Ok(written)
}

/// Stable, filesystem-safe instance id for a modpack: `modpack-<project>-<mc>`.
/// Keyed on the (opaque, unique) Modrinth project id so reinstalling the same
/// pack reuses the same instance, and two different packs never collide.
pub fn instance_id(project_id: &str, mc_version: &str) -> String {
    let proj: String = project_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mc: String = mc_version
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '-' })
        .collect();
    format!("modpack-{proj}-{mc}")
}

fn client_wanted(f: &IndexFile) -> bool {
    // Install required + optional client files; skip ones marked unsupported.
    !matches!(f.env.as_ref().and_then(|e| e.client.as_deref()), Some("unsupported"))
}

fn short_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Join `rel` onto `base`, refusing anything that could escape `base`
/// (absolute paths, `..`, drive/root prefixes). Forward and back slashes are
/// both treated as separators on Windows.
fn safe_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    let mut out = base.to_path_buf();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => bail!("unsafe '..' path in modpack: {rel}"),
            Component::Prefix(_) | Component::RootDir => {
                bail!("unsafe absolute path in modpack: {rel}")
            }
        }
    }
    Ok(out)
}

async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha1: Option<&str>,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("requesting {url}"))?
        .error_for_status()?;
    let tmp = dest.with_extension("part");
    let mut out = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = resp.chunk().await? {
        hasher.update(&chunk);
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    drop(out);
    if let Some(expected) = sha1 {
        let got = hex::encode(hasher.finalize());
        if got != expected {
            let _ = tokio::fs::remove_file(&tmp).await;
            bail!("hash mismatch for {}: got {got}, want {expected}", dest.display());
        }
    }
    tokio::fs::rename(&tmp, dest).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_with(dependencies: &[(&str, &str)]) -> ModpackIndex {
        ModpackIndex {
            format_version: 1,
            name: String::new(),
            version_id: String::new(),
            dependencies: dependencies
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            files: Vec::new(),
        }
    }

    #[test]
    fn loader_requirement_uses_declared_fabric_loader() {
        let index = index_with(&[("minecraft", "1.20.1"), ("fabric-loader", "0.15.11")]);
        let req = loader_requirement(&index).unwrap();

        assert_eq!(req.mc_version, "1.20.1");
        assert_eq!(req.loader_version, "0.15.11");
    }

    #[test]
    fn loader_requirement_rejects_non_fabric_pack() {
        let index = index_with(&[("minecraft", "1.20.1"), ("forge", "47.3.0")]);

        assert!(loader_requirement(&index).is_err());
    }
}
