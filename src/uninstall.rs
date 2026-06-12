use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

/// `tinuxlauncher uninstall` - removes the binary (and on Windows the PATH
/// entry the installer added), with an opt-in wipe of all game data.
pub fn cli_uninstall() -> Result<()> {
    let exe = std::env::current_exe().context("locating current exe")?;
    println!("Tinux Launcher v{}", env!("CARGO_PKG_VERSION"));
    println!("  binary: {}", exe.display());
    if !confirm("Remove Tinux Launcher? [y/N] ")? {
        println!("Nothing removed.");
        return Ok(());
    }

    if let Some(dirs) = directories::ProjectDirs::from("dev", "tinux", "TinuxLauncher") {
        println!("Game data (worlds, instances, mods, settings, logs):");
        println!("  {}", dirs.data_dir().display());
        if confirm("Also delete game data? [y/N] ")? {
            // a full wipe includes the saved Microsoft session
            crate::auth::logout();
            remove_tree(dirs.data_dir());
            remove_tree(dirs.cache_dir());
            println!("Game data deleted.");
        } else {
            println!("Game data kept (a reinstall picks it up again).");
        }
    }

    remove_binary(&exe)
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading answer")?;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

/// Delete a tree, then prune now-empty parent shells (the `directories`
/// layout nests data under vendor folders, e.g. ...\tinux\TinuxLauncher\data).
fn remove_tree(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
    let mut p = dir.parent();
    while let Some(parent) = p {
        if std::fs::remove_dir(parent).is_err() {
            break; // not empty - stop pruning
        }
        p = parent.parent();
    }
}

/// A running exe can't be deleted on Windows, so a hidden helper waits for
/// this process to exit, deletes the binary, and drops the PATH entry.
#[cfg(windows)]
fn remove_binary(exe: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let dir = exe.parent().context("exe has no parent dir")?;
    // Delete the whole folder (and its PATH entry) only when it's the
    // dedicated dir the installer created; an exe parked in a shared bin
    // dir loses just itself, and its PATH entry is none of our business.
    let dedicated = dir.file_name().and_then(|s| s.to_str()) == Some("TinuxLauncher");
    let target = if dedicated { dir } else { exe };

    let target_q = target.display().to_string().replace('\'', "''");
    let pid = std::process::id();
    let path_cleanup = if dedicated {
        let dir_q = dir.display().to_string().replace('\'', "''");
        format!(
            "$dir = '{dir_q}'\r\n\
             $path = [Environment]::GetEnvironmentVariable('Path', 'User')\r\n\
             if ($null -ne $path) {{\r\n\
             \x20\x20$parts = $path -split ';' | Where-Object {{ $_ -and $_ -ne $dir }}\r\n\
             \x20\x20[Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')\r\n\
             }}\r\n"
        )
    } else {
        String::new()
    };
    let script = format!(
        "$ErrorActionPreference = 'SilentlyContinue'\r\n\
         Wait-Process -Id {pid} -Timeout 60\r\n\
         for ($i = 0; $i -lt 30; $i++) {{\r\n\
         \x20\x20Remove-Item -Recurse -Force -LiteralPath '{target_q}'\r\n\
         \x20\x20if (-not (Test-Path -LiteralPath '{target_q}')) {{ break }}\r\n\
         \x20\x20Start-Sleep -Milliseconds 500\r\n\
         }}\r\n\
         {path_cleanup}"
    );
    let script_path = std::env::temp_dir().join("tinux-launcher-uninstall.ps1");
    std::fs::write(&script_path, script.as_bytes())
        .context("writing uninstall helper script")?;

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
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("spawning uninstall helper")?;

    println!("Tinux Launcher uninstalled. The binary is removed right after this process exits.");
    Ok(())
}

/// Unix can unlink a running binary, so everything happens in-process.
#[cfg(unix)]
fn remove_binary(exe: &Path) -> Result<()> {
    // the installer drops a `tinux` symlink next to the binary
    if let Some(dir) = exe.parent() {
        let link = dir.join("tinux");
        if let Ok(target) = std::fs::read_link(&link) {
            if target.file_name() == exe.file_name() {
                let _ = std::fs::remove_file(&link);
            }
        }
    }
    std::fs::remove_file(exe).with_context(|| format!("deleting {}", exe.display()))?;
    println!("Tinux Launcher uninstalled.");
    Ok(())
}
