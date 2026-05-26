use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

const API_BASE: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub author: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    hits: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProjectVersion {
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    version_type: String,
    files: Vec<VersionFile>,
}

#[derive(Debug, Deserialize)]
struct VersionFile {
    url: String,
    filename: String,
    #[serde(default)]
    hashes: Hashes,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Default, Deserialize)]
struct Hashes {
    #[serde(default)]
    sha1: Option<String>,
}

pub async fn search(
    client: &reqwest::Client,
    query: &str,
    mc_version: &str,
    loader: &str,
) -> Result<Vec<SearchHit>> {
    let facets = format!(
        r#"[["categories:{loader}"],["versions:{mc_version}"],["project_type:mod"]]"#
    );
    let url = format!("{API_BASE}/search");
    let resp: SearchResponse = client
        .get(&url)
        .query(&[
            ("query", query),
            ("facets", &facets),
            ("limit", "20"),
            ("index", "relevance"),
        ])
        .send()
        .await
        .context("contacting Modrinth search")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Modrinth search response")?;
    Ok(resp.hits)
}

/// Download the latest compatible primary file for a project to `mods_dir`.
/// Returns the saved filename on success.
pub async fn install_latest(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    mc_version: &str,
    loader: &str,
    mods_dir: &Path,
) -> Result<String> {
    let url = format!("{API_BASE}/project/{project_id_or_slug}/version");
    let loaders = format!(r#"["{loader}"]"#);
    let game_versions = format!(r#"["{mc_version}"]"#);
    let versions: Vec<ProjectVersion> = client
        .get(&url)
        .query(&[("loaders", loaders.as_str()), ("game_versions", game_versions.as_str())])
        .send()
        .await
        .context("contacting Modrinth versions endpoint")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Modrinth version list")?;

    let chosen = versions
        .iter()
        .find(|v| v.version_type == "release")
        .or_else(|| versions.first())
        .ok_or_else(|| anyhow!("no compatible Modrinth version found for {mc_version}/{loader}"))?;

    let file = chosen
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| chosen.files.first())
        .ok_or_else(|| anyhow!("Modrinth version has no files"))?;

    fs::create_dir_all(mods_dir).await?;
    let dest = mods_dir.join(&file.filename);

    let mut resp = client
        .get(&file.url)
        .send()
        .await
        .with_context(|| format!("downloading {}", file.url))?
        .error_for_status()?;

    let tmp = dest.with_extension("part");
    let mut out = fs::File::create(&tmp).await?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = resp.chunk().await? {
        hasher.update(&chunk);
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    drop(out);

    if let Some(expected) = &file.hashes.sha1 {
        let got = hex::encode(hasher.finalize());
        if &got != expected {
            let _ = fs::remove_file(&tmp).await;
            anyhow::bail!(
                "hash mismatch for {}: got {got}, want {expected}",
                file.filename
            );
        }
    }
    fs::rename(&tmp, &dest).await?;
    Ok(file.filename.clone())
}

pub async fn delete(mods_dir: &Path, filename: &str) -> Result<()> {
    if filename.contains('/') || filename.contains('\\') {
        anyhow::bail!("invalid mod filename");
    }
    let target = mods_dir.join(filename);
    fs::remove_file(&target)
        .await
        .with_context(|| format!("removing {}", target.display()))?;
    Ok(())
}
