use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ms_client_id: Option<String>,
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
    // Matches `paths.rs`. See the note there for why this keeps the legacy "revo"
    // segments even after the Tinux rebrand.
    directories::ProjectDirs::from("dev", "revo", "RevoLauncher")
        .map(|d| d.data_dir().join("config.json"))
}

/// Make sure a `config.json` exists so users have something to open and edit.
/// Existing files are left alone.
pub fn ensure_stub() {
    let Some(p) = path() else { return };
    if p.exists() {
        return;
    }
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let stub = serde_json::json!({
        "_help": "Paste your Microsoft Entra (Azure) Application (client) ID below.",
        "_docs": "See README.md → 'Microsoft sign-in' for the 5-minute Azure setup.",
        "ms_client_id": null
    });
    let _ = std::fs::write(&p, serde_json::to_vec_pretty(&stub).unwrap_or_default());
}

#[cfg(windows)]
pub fn open_with_default_app(p: &Path) -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &p.to_string_lossy()])
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
pub fn open_with_default_app(p: &Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(p).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn open_with_default_app(p: &Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(p).spawn().map(|_| ())
}
