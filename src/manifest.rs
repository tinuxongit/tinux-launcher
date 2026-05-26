use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<ManifestVersion>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestVersion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: VersionKind,
    pub url: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VersionKind {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
}

impl VersionKind {
    pub fn label(self) -> &'static str {
        match self {
            VersionKind::Release => "release",
            VersionKind::Snapshot => "snapshot",
            VersionKind::OldBeta => "beta",
            VersionKind::OldAlpha => "alpha",
        }
    }
}

pub async fn fetch(client: &reqwest::Client) -> Result<VersionManifest> {
    let bytes = client
        .get(MANIFEST_URL)
        .send()
        .await
        .context("fetching version manifest")?
        .error_for_status()
        .context("version manifest http status")?
        .bytes()
        .await
        .context("reading version manifest body")?;
    let manifest: VersionManifest =
        serde_json::from_slice(&bytes).context("parsing version manifest")?;
    Ok(manifest)
}
