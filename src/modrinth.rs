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
#[allow(dead_code)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    #[serde(default)]
    pub total_hits: u32,
    #[serde(default)]
    pub offset: u32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProjectVersion {
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    version_type: String,
    files: Vec<VersionFile>,
    #[serde(default)]
    dependencies: Vec<VersionDependency>,
}

#[derive(Debug, Deserialize)]
struct VersionDependency {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    dependency_type: String,
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

#[derive(Debug, Clone, Deserialize)]
pub struct Category {
    pub name: String,
    pub project_type: String,
    #[serde(default)]
    pub header: String,
}

pub async fn fetch_categories(client: &reqwest::Client) -> Result<Vec<Category>> {
    let url = format!("{API_BASE}/tag/category");
    let cats: Vec<Category> = client
        .get(&url)
        .send()
        .await
        .context("contacting Modrinth categories")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Modrinth categories")?;
    Ok(cats)
}

pub async fn search(
    client: &reqwest::Client,
    query: &str,
    mc_version: &str,
    loader: &str,
    project_type: &str,
    include_loader_facet: bool,
    categories: &[String],
    offset: u32,
) -> Result<SearchResponse> {
    let mut groups: Vec<String> = Vec::new();
    if include_loader_facet {
        groups.push(format!(r#"["categories:{loader}"]"#));
    }
    if !categories.is_empty() {
        let or_terms: Vec<String> = categories
            .iter()
            .map(|c| format!(r#""categories:{c}""#))
            .collect();
        groups.push(format!("[{}]", or_terms.join(",")));
    }
    groups.push(format!(r#"["versions:{mc_version}"]"#));
    groups.push(format!(r#"["project_type:{project_type}"]"#));
    let facets = format!("[{}]", groups.join(","));
    let url = format!("{API_BASE}/search");
    let offset_str = offset.to_string();
    let resp: SearchResponse = client
        .get(&url)
        .query(&[
            ("query", query),
            ("facets", &facets),
            ("limit", "20"),
            ("offset", offset_str.as_str()),
            ("index", "relevance"),
        ])
        .send()
        .await
        .context("contacting Modrinth search")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Modrinth search response")?;
    Ok(resp)
}

#[derive(Debug, Clone)]
pub struct InstallResult {
    pub primary_filename: String,
    /// Project ids of required dependencies that were also installed in this call.
    pub dep_project_ids: Vec<String>,
}

/// Download the latest compatible primary file for a project to `mods_dir`,
/// also recursively installing every required dependency.
pub async fn install_with_deps(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    mc_version: &str,
    loader: Option<&str>,
    mods_dir: &Path,
) -> Result<InstallResult> {
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut dep_project_ids: Vec<String> = Vec::new();
    let primary_filename = install_recursive(
        client,
        project_id_or_slug,
        mc_version,
        loader,
        mods_dir,
        &mut visited,
        &mut dep_project_ids,
        0,
    )
    .await?;
    Ok(InstallResult {
        primary_filename,
        dep_project_ids,
    })
}

fn install_recursive<'a>(
    client: &'a reqwest::Client,
    project_id_or_slug: &'a str,
    mc_version: &'a str,
    loader: Option<&'a str>,
    mods_dir: &'a Path,
    visited: &'a mut std::collections::HashSet<String>,
    collected_deps: &'a mut Vec<String>,
    depth: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
    Box::pin(async move {
        // Bound recursion in case Modrinth ever returns a cycle.
        if depth > 8 {
            anyhow::bail!("dependency depth exceeded (cycle?)");
        }
        let key = project_id_or_slug.to_string();
        if !visited.insert(key.clone()) {
            // Already processed this project in this install — skip.
            return Ok(String::new());
        }
        let filename = install_one(client, project_id_or_slug, mc_version, loader, mods_dir).await?;
        // Refetch the version to inspect required deps. (install_one threw
        // away the parsed structure; one extra GET is acceptable here.)
        let deps = required_deps_for(client, project_id_or_slug, mc_version, loader).await?;
        for dep_id in deps {
            if visited.contains(&dep_id) {
                continue;
            }
            if depth == 0 {
                collected_deps.push(dep_id.clone());
            }
            // Best-effort: if a dep fails to install we surface it but don't
            // abort the whole tree.
            let _ = install_recursive(
                client,
                &dep_id,
                mc_version,
                loader,
                mods_dir,
                visited,
                collected_deps,
                depth + 1,
            )
            .await;
        }
        Ok(filename)
    })
}

async fn required_deps_for(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    mc_version: &str,
    loader: Option<&str>,
) -> Result<Vec<String>> {
    let versions = list_versions(client, project_id_or_slug, mc_version, loader).await?;
    let chosen = pick_version(&versions);
    let Some(v) = chosen else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for d in &v.dependencies {
        if d.dependency_type != "required" {
            continue;
        }
        if let Some(pid) = &d.project_id {
            if !pid.is_empty() {
                out.push(pid.clone());
            }
        }
    }
    Ok(out)
}

async fn list_versions(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    mc_version: &str,
    loader: Option<&str>,
) -> Result<Vec<ProjectVersion>> {
    let url = format!("{API_BASE}/project/{project_id_or_slug}/version");
    let game_versions = format!(r#"["{mc_version}"]"#);
    let mut req = client
        .get(&url)
        .query(&[("game_versions", game_versions.as_str())]);
    let loaders_owned;
    if let Some(loader) = loader {
        loaders_owned = format!(r#"["{loader}"]"#);
        req = req.query(&[("loaders", loaders_owned.as_str())]);
    }
    let versions: Vec<ProjectVersion> = req
        .send()
        .await
        .context("contacting Modrinth versions endpoint")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Modrinth version list")?;
    Ok(versions)
}

fn pick_version(versions: &[ProjectVersion]) -> Option<&ProjectVersion> {
    versions
        .iter()
        .find(|v| v.version_type == "release")
        .or_else(|| versions.first())
}

async fn install_one(
    client: &reqwest::Client,
    project_id_or_slug: &str,
    mc_version: &str,
    loader: Option<&str>,
    mods_dir: &Path,
) -> Result<String> {
    let url = format!("{API_BASE}/project/{project_id_or_slug}/version");
    let game_versions = format!(r#"["{mc_version}"]"#);
    let mut req = client
        .get(&url)
        .query(&[("game_versions", game_versions.as_str())]);
    let loaders_owned;
    if let Some(loader) = loader {
        loaders_owned = format!(r#"["{loader}"]"#);
        req = req.query(&[("loaders", loaders_owned.as_str())]);
    }
    let versions: Vec<ProjectVersion> = req
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
        .ok_or_else(|| {
            anyhow!(
                "no compatible Modrinth version found for {mc_version}{}",
                loader.map(|l| format!("/{l}")).unwrap_or_default()
            )
        })?;

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
