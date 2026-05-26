use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ms_client_id: Option<String>,
    #[serde(default)]
    pub offline_name: Option<String>,
    #[serde(default)]
    pub offline_skin_url: Option<String>,
    #[serde(default)]
    pub last_played_version: Option<String>,
    #[serde(default)]
    pub last_filter: Option<String>,
    /// Max heap in MB used for `-Xmx`. Min is fixed at 512MB.
    #[serde(default)]
    pub max_ram_mb: Option<u32>,
    /// Optional override path for the Java executable. Empty/None = auto-detect.
    #[serde(default)]
    pub java_path: Option<String>,
    /// Per-version Java overrides keyed on the manifest version id.
    /// A value here wins over `java_path` and over auto-detection.
    #[serde(default)]
    pub java_path_per_version: HashMap<String, String>,
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}

pub fn path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "tinux", "TinuxLauncher")
        .map(|d| d.data_dir().join("config.json"))
}

pub fn save_offline_name(name: &str) {
    update(|c| c.offline_name = Some(name.to_string()));
}

pub fn save_offline_skin_url(url: &str) {
    update(|c| c.offline_skin_url = Some(url.to_string()));
}

pub fn save_last_played(version: &str, filter: &str) {
    update(|c| {
        c.last_played_version = Some(version.to_string());
        c.last_filter = Some(filter.to_string());
    });
}

pub fn save_max_ram(mb: u32) {
    update(|c| c.max_ram_mb = Some(mb));
}

pub fn save_java_path(path: &str) {
    update(|c| {
        c.java_path = if path.trim().is_empty() {
            None
        } else {
            Some(path.to_string())
        };
    });
}

pub fn save_java_path_for(version_id: &str, path: &str) {
    update(|c| {
        if path.trim().is_empty() {
            c.java_path_per_version.remove(version_id);
        } else {
            c.java_path_per_version
                .insert(version_id.to_string(), path.to_string());
        }
    });
}

fn update(f: impl FnOnce(&mut Config)) {
    let Some(p) = path() else { return };
    let mut cfg = Config::load(&p);
    f(&mut cfg);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(&cfg) {
        let _ = std::fs::write(&p, json);
    }
}

pub fn ensure_stub() {
    let Some(p) = path() else { return };
    if p.exists() {
        return;
    }
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let stub = serde_json::json!({ "ms_client_id": null });
    let _ = std::fs::write(&p, serde_json::to_vec_pretty(&stub).unwrap_or_default());
}

