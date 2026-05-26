use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ms_client_id: Option<String>,
    #[serde(default)]
    pub offline_name: Option<String>,
    #[serde(default)]
    pub offline_skin_url: Option<String>,
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

