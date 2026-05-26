use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;

use crate::event::WorkerMsg;

const REPO_OWNER: &str = "tinuxongit";
const REPO_NAME: &str = "tinux-launcher";
const LATEST_REDIRECT_URL: &str =
    "https://github.com/tinuxongit/tinux-launcher/releases/latest";

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

pub async fn check(_client: &reqwest::Client) -> Result<UpdateInfo> {
    let current = env!("CARGO_PKG_VERSION").to_string();

    // Use the website redirect (not the REST API) to dodge the 60/hr unauthenticated
    // rate limit. GET /releases/latest 302s to /releases/tag/<tag>.
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("tinux-launcher/{current}"))
        .build()
        .context("building update HTTP client")?;
    let resp = no_redirect
        .get(LATEST_REDIRECT_URL)
        .send()
        .await
        .context("fetching latest release redirect")?;
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow!("no Location header on releases/latest"))?
        .to_string();

    // Location looks like: https://github.com/owner/repo/releases/tag/v0.1.4
    let tag = location
        .rsplit('/')
        .next()
        .ok_or_else(|| anyhow!("could not parse tag from {location}"))?
        .to_string();
    if tag.is_empty() || tag == "latest" {
        anyhow::bail!("redirect didn't point at a tagged release: {location}");
    }
    let latest = tag.trim_start_matches('v').to_string();
    let up_to_date = latest == current;
    let html_url = format!(
        "https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/{tag}"
    );
    let asset = if up_to_date {
        None
    } else {
        asset_for_current_platform(&tag)
    };
    Ok(UpdateInfo {
        current,
        latest,
        html_url,
        up_to_date,
        asset,
    })
}

fn asset_for_current_platform(tag: &str) -> Option<ReleaseAsset> {
    let name: &str = if cfg!(target_os = "windows") {
        "tinux-launcher-windows-x64.exe"
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "tinux-launcher-macos-arm64"
        } else {
            "tinux-launcher-macos-x64"
        }
    } else if cfg!(target_os = "linux") {
        "tinux-launcher-linux-x64"
    } else {
        return None;
    };
    let url = format!(
        "https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/download/{tag}/{name}"
    );
    Some(ReleaseAsset {
        name: name.to_string(),
        url,
        size: 0,
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
    let log_path = script_dir.join("tinux-launcher-update.log");

    let src = new_exe.display().to_string().replace('\'', "''");
    let dst = current.display().to_string().replace('\'', "''");
    let log = log_path.display().to_string().replace('\'', "''");
    // Use cmd.exe's `start` to launch the new exe in its own console window —
    // Start-Process detaches console apps in a way that leaves the TUI with
    // nowhere to render. `start "" "<path>"` is the canonical pattern.
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\r\n\
         $log = '{log}'\r\n\
         function Log($m) {{ \"[$([DateTime]::Now.ToString('HH:mm:ss'))] $m\" | Out-File -Append -FilePath $log -Encoding utf8 }}\r\n\
         Log 'helper started'\r\n\
         Start-Sleep -Seconds 2\r\n\
         $src = '{src}'\r\n\
         $dst = '{dst}'\r\n\
         Log \"src=$src\"\r\n\
         Log \"dst=$dst\"\r\n\
         for ($i = 0; $i -lt 30; $i++) {{\r\n\
         \x20\x20  Move-Item -Force -LiteralPath $src -Destination $dst -ErrorAction SilentlyContinue\r\n\
         \x20\x20  if ((Test-Path $dst -PathType Leaf) -and -not (Test-Path $src)) {{ Log \"moved on try $i\"; break }}\r\n\
         \x20\x20  Start-Sleep -Milliseconds 500\r\n\
         }}\r\n\
         if (Test-Path $src) {{ Log 'WARNING: move never succeeded; launching old location instead' }}\r\n\
         Log 'launching new exe via ShellExecute'\r\n\
         try {{\r\n\
         \x20\x20  $psi = New-Object System.Diagnostics.ProcessStartInfo\r\n\
         \x20\x20  $psi.FileName = $dst\r\n\
         \x20\x20  $psi.UseShellExecute = $true\r\n\
         \x20\x20  $psi.WindowStyle = 'Normal'\r\n\
         \x20\x20  $proc = [System.Diagnostics.Process]::Start($psi)\r\n\
         \x20\x20  if ($proc) {{ Log \"launched, pid=$($proc.Id)\" }} else {{ Log 'Start returned $null' }}\r\n\
         }} catch {{ Log \"launch error: $_\" }}\r\n\
         Log 'helper done'\r\n"
    );
    std::fs::write(&script_path, script.as_bytes())
        .context("writing update helper script")?;

    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW alone — DETACHED_PROCESS makes CREATE_NO_WINDOW a no-op
    // and PowerShell with no console at all can exit silently before reaching
    // the first script statement.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // Record the spawn outcome from Rust's side too, so we can tell whether
    // the helper was invoked at all even if PowerShell itself never runs.
    let rust_log_path = script_dir.join("tinux-launcher-update-rust.log");
    let mut rust_log = String::new();
    rust_log.push_str(&format!("script: {}\r\n", script_path.display()));
    rust_log.push_str(&format!("log:    {}\r\n", log_path.display()));
    rust_log.push_str(&format!("src:    {}\r\n", new_exe.display()));
    rust_log.push_str(&format!("dst:    {}\r\n", current.display()));

    let spawned = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &script_path.display().to_string(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match &spawned {
        Ok(child) => rust_log.push_str(&format!("spawn: ok, pid={}\r\n", child.id())),
        Err(e) => rust_log.push_str(&format!("spawn: ERR {e}\r\n")),
    }
    let _ = std::fs::write(&rust_log_path, rust_log);
    spawned.context("spawning update helper")?;
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
