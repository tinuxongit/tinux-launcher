use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::event::WorkerMsg;

const RELEASES_URL: &str =
    "https://api.github.com/repos/tinuxongit/tinux-launcher/releases/latest";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub html_url: String,
    pub up_to_date: bool,
    pub asset: Option<ReleaseAsset>,
}

#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

pub async fn check(client: &reqwest::Client) -> Result<UpdateInfo> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let resp = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("contacting GitHub releases API")?
        .error_for_status()
        .context("GitHub releases status")?;
    let rel: GhRelease = resp.json().await.context("parsing release JSON")?;
    let latest = rel.tag_name.trim_start_matches('v').to_string();
    let up_to_date = latest == current;
    let asset = pick_asset(&rel.assets);
    Ok(UpdateInfo {
        current,
        latest,
        html_url: rel.html_url,
        up_to_date,
        asset,
    })
}

fn pick_asset(assets: &[GhAsset]) -> Option<ReleaseAsset> {
    let want_windows = cfg!(target_os = "windows");
    let want_macos = cfg!(target_os = "macos");
    let want_linux = cfg!(target_os = "linux");
    let pick = assets.iter().find(|a| {
        let n = a.name.to_lowercase();
        if want_windows {
            n.contains("windows") || n.ends_with(".exe")
        } else if want_macos {
            n.contains("darwin") || n.contains("macos") || n.contains("apple")
        } else if want_linux {
            n.contains("linux")
        } else {
            false
        }
    })?;
    Some(ReleaseAsset {
        name: pick.name.clone(),
        url: pick.browser_download_url.clone(),
        size: pick.size,
    })
}

/// Download `asset` to a temp file. Streams progress through `tx`.
/// Returns the path of the downloaded file.
pub async fn download_asset(
    client: &reqwest::Client,
    asset: &ReleaseAsset,
    cache_dir: &Path,
    tx: UnboundedSender<WorkerMsg>,
) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir).await?;
    let dest = cache_dir.join(format!("update-{}", asset.name));
    let tmp = dest.with_extension("part");
    let mut resp = client
        .get(&asset.url)
        .send()
        .await
        .with_context(|| format!("downloading {}", asset.url))?
        .error_for_status()?;
    let total = resp.content_length().unwrap_or(asset.size);
    let done = Arc::new(AtomicU64::new(0));
    let mut file = fs::File::create(&tmp).await?;
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk).await?;
        let d = done.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;
        let _ = tx.send(WorkerMsg::UpdateDownloadProgress { done: d, total });
    }
    file.flush().await?;
    drop(file);
    if let Err(e) = fs::rename(&tmp, &dest).await {
        // On Windows, rename can fail if dest exists; try remove + retry.
        let _ = fs::remove_file(&dest).await;
        fs::rename(&tmp, &dest)
            .await
            .with_context(|| format!("renaming downloaded update: {e}"))?;
    }
    Ok(dest)
}

/// Hand off to a small helper that swaps the binary and relaunches.
/// Returns Ok(()) once the helper has been spawned — the caller should
/// immediately exit so the helper can replace the file.
#[cfg(windows)]
pub fn spawn_swap_and_restart(new_exe: &Path) -> Result<()> {
    let current = std::env::current_exe().context("locating current exe")?;
    let script_dir = std::env::temp_dir();
    let script_path = script_dir.join("tinux-launcher-update.ps1");

    let src = new_exe.display().to_string().replace('\'', "''");
    let dst = current.display().to_string().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\r\n\
         Start-Sleep -Seconds 2\r\n\
         $src = '{src}'\r\n\
         $dst = '{dst}'\r\n\
         for ($i = 0; $i -lt 30; $i++) {{\r\n\
         \x20\x20  Move-Item -Force -LiteralPath $src -Destination $dst -ErrorAction SilentlyContinue\r\n\
         \x20\x20  if (Test-Path $dst -PathType Leaf) {{ if (-not (Test-Path $src)) {{ break }} }}\r\n\
         \x20\x20  Start-Sleep -Milliseconds 500\r\n\
         }}\r\n\
         Start-Process -FilePath $dst\r\n"
    );
    std::fs::write(&script_path, script.as_bytes())
        .context("writing update helper script")?;

    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script_path.display().to_string(),
        ])
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning update helper")?;
    Ok(())
}

#[cfg(not(windows))]
pub fn spawn_swap_and_restart(_new_exe: &Path) -> Result<()> {
    anyhow::bail!("auto-install on this platform isn't supported yet — open the release page instead")
}

pub fn spawn_check(client: reqwest::Client, tx: UnboundedSender<WorkerMsg>) {
    let _ = tx.send(WorkerMsg::UpdateCheckStarted);
    tokio::spawn(async move {
        match check(&client).await {
            Ok(info) => {
                let _ = tx.send(WorkerMsg::UpdateCheckResult(info));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::UpdateCheckFailed(format!("{e:#}")));
            }
        }
    });
}

pub fn spawn_download(
    client: reqwest::Client,
    asset: ReleaseAsset,
    cache_dir: PathBuf,
    tx: UnboundedSender<WorkerMsg>,
) {
    let _ = tx.send(WorkerMsg::UpdateDownloadStarted);
    tokio::spawn(async move {
        match download_asset(&client, &asset, &cache_dir, tx.clone()).await {
            Ok(path) => {
                let _ = tx.send(WorkerMsg::UpdateDownloaded(path));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::UpdateDownloadFailed(format!("{e:#}")));
            }
        }
    });
}
