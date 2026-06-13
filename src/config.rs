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
    /// Selected MC version per filter tab, so each loader's tab restores
    /// independently. Keyed by `VersionFilter::as_str()`. New loaders plug
    /// in here automatically.
    #[serde(default)]
    pub selections_by_filter: HashMap<String, String>,
    #[serde(default)]
    pub show_snapshots: bool,
    #[serde(default)]
    pub show_older: bool,
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
    /// Mod loader selected on the Modded tab: "fabric" | "neoforge" | "forge".
    #[serde(default)]
    pub loader: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}

/// A modpack installed as its own launchable instance. `id` is both the
/// version id (`versions/<id>/`) and the instance dir (`instances/<id>/`).
/// The registry lives in `modpacks.json` next to `config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackInstance {
    pub id: String,
    pub name: String,
    pub mc_version: String,
    #[serde(default)]
    pub modpack_version: String,
    #[serde(default)]
    pub project_id: String,
    /// "fabric" | "forge" | "neoforge". Empty in registries written before
    /// loader support landed; those were all Fabric.
    #[serde(default)]
    pub loader: String,
}

fn modpacks_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "tinux", "TinuxLauncher")
        .map(|d| d.data_dir().join("modpacks.json"))
}

pub fn load_modpacks() -> Vec<ModpackInstance> {
    let Some(p) = modpacks_path() else { return Vec::new() };
    std::fs::read(&p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_modpacks(list: &[ModpackInstance]) {
    let Some(p) = modpacks_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(&p, json);
    }
}

pub fn add_modpack(entry: ModpackInstance) {
    let mut list = load_modpacks();
    list.retain(|e| e.id != entry.id); // re-install replaces the old record
    list.push(entry);
    save_modpacks(&list);
}

pub fn remove_modpack(id: &str) {
    let mut list = load_modpacks();
    list.retain(|e| e.id != id);
    save_modpacks(&list);
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
        c.selections_by_filter
            .insert(filter.to_string(), version.to_string());
    });
}

pub fn save_selection(filter: &str, version: &str) {
    update(|c| {
        c.selections_by_filter
            .insert(filter.to_string(), version.to_string());
        c.last_filter = Some(filter.to_string());
    });
}

pub fn save_active_filter(filter: &str) {
    update(|c| c.last_filter = Some(filter.to_string()));
}

pub fn save_loader(loader: &str) {
    update(|c| c.loader = Some(loader.to_string()));
}

pub fn save_version_toggles(show_snapshots: bool, show_older: bool) {
    update(|c| {
        c.show_snapshots = show_snapshots;
        c.show_older = show_older;
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

