//! Mojang's own Java runtimes, downloaded on demand.
//!
//! Every version JSON names the Java it wants (`javaVersion`), and Mojang
//! publishes a matching JRE per platform. When the machine hasn't got that
//! Java, this fetches it into `runtimes/<component>/` rather than telling the
//! player to go and install a JDK by hand. `java.rs` looks in that folder, so
//! a runtime downloaded once is found by every later launch.
//!
//! The index is a two-step lookup: `all.json` maps platform -> component ->
//! a manifest URL, and that manifest lists every file in the runtime with its
//! sha1. Downloading is the ordinary `download::run_jobs` pass, so runtimes
//! get the same hashing, parallelism, progress and cancellation as anything
//! else.

use crate::download::{run_jobs, CancelFlag, DownloadJob, ProgressEvent, VerifyMode};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

const ALL_JSON: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

#[derive(Debug, Deserialize)]
struct Available {
    manifest: ManifestRef,
    version: RuntimeVersion,
}

#[derive(Debug, Deserialize)]
struct ManifestRef {
    url: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeVersion {
    name: String,
}

#[derive(Debug, Deserialize)]
struct FileManifest {
    files: HashMap<String, Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Entry {
    Directory,
    File {
        downloads: EntryDownloads,
        #[serde(default)]
        executable: bool,
    },
    Link {
        target: String,
    },
}

#[derive(Debug, Deserialize)]
struct EntryDownloads {
    raw: RawDownload,
}

#[derive(Debug, Deserialize)]
struct RawDownload {
    url: String,
    sha1: String,
    size: u64,
}

/// A runtime Mojang publishes for this machine.
#[derive(Debug, Clone)]
pub struct RuntimeChoice {
    /// Mojang's name for it, e.g. `java-runtime-epsilon`. Also the folder.
    pub component: String,
    /// Human version, e.g. `25.0.1`.
    pub version: String,
    manifest_url: String,
}

/// Mojang's key for this OS and CPU, or None where they publish nothing.
///
/// Linux on ARM is the gap that matters: Mojang ships `linux` (x86_64) and
/// `linux-i386` and no 64-bit ARM build at all, so a Raspberry Pi or an ARM
/// server has to install its own JDK.
pub fn platform_key() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        match std::env::consts::ARCH {
            "x86_64" => Some("windows-x64"),
            "x86" => Some("windows-x86"),
            "aarch64" => Some("windows-arm64"),
            _ => None,
        }
    } else if cfg!(target_os = "macos") {
        match std::env::consts::ARCH {
            "x86_64" => Some("mac-os"),
            "aarch64" => Some("mac-os-arm64"),
            _ => None,
        }
    } else if cfg!(target_os = "linux") {
        match std::env::consts::ARCH {
            "x86_64" => Some("linux"),
            "x86" => Some("linux-i386"),
            _ => None,
        }
    } else {
        None
    }
}

/// The major version out of a runtime's version name: `25.0.1` -> 25, and
/// `8u202` -> 8, which is how Mojang writes the Java 8 runtime.
fn major_of(name: &str) -> Option<u32> {
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Every number in a version name, so `17.0.15` sorts above `17.0.9` the way
/// a plain string compare would not.
fn version_sort_key(name: &str) -> Vec<u32> {
    name.split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Find the runtime for `major` on this machine.
///
/// `component` is the name the version JSON asked for and wins when Mojang
/// still publishes it. Otherwise any component of the right major does, newest
/// first, because several components share a major (`java-runtime-gamma` and
/// `java-runtime-beta` are both 17).
pub async fn find(
    client: &reqwest::Client,
    major: u32,
    component: Option<&str>,
) -> Result<RuntimeChoice> {
    let platform = platform_key().ok_or_else(|| {
        anyhow!(
            "Mojang publishes no Java runtime for {} on {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    let index: HashMap<String, HashMap<String, Vec<Available>>> = client
        .get(ALL_JSON)
        .send()
        .await
        .context("fetching Java runtime index")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Java runtime index")?;

    let components = index
        .get(platform)
        .ok_or_else(|| anyhow!("Java runtime index has nothing for {platform}"))?;

    // Sorted ascending so the last one is the pick: a component the version
    // JSON named outranks everything, and between equals the newest wins.
    let mut candidates: Vec<(bool, Vec<u32>, &String, &Available)> = components
        .iter()
        .filter_map(|(name, entries)| {
            let entry = entries.first()?;
            (major_of(&entry.version.name) == Some(major)).then_some((
                Some(name.as_str()) == component,
                version_sort_key(&entry.version.name),
                name,
                entry,
            ))
        })
        .collect();
    candidates.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));

    let (_, _, name, entry) = candidates.pop().ok_or_else(|| {
        anyhow!("Mojang publishes no Java {major} runtime for {platform}")
    })?;

    Ok(RuntimeChoice {
        component: name.clone(),
        version: entry.version.name.clone(),
        manifest_url: entry.manifest.url.clone(),
    })
}

/// Download a runtime into `runtimes/<component>/` and return its `java`.
///
/// Directories come first, then every file through the normal download pass,
/// then the executable bits and symlinks, because a link's target has to
/// exist before the link is made.
pub async fn install(
    client: &reqwest::Client,
    runtimes_dir: &Path,
    choice: &RuntimeChoice,
    progress: &UnboundedSender<ProgressEvent>,
    cancel: &CancelFlag,
) -> Result<PathBuf> {
    let manifest: FileManifest = client
        .get(&choice.manifest_url)
        .send()
        .await
        .context("fetching Java runtime manifest")?
        .error_for_status()?
        .json()
        .await
        .context("parsing Java runtime manifest")?;

    let dest = runtimes_dir.join(&choice.component);

    for (path, entry) in &manifest.files {
        if matches!(entry, Entry::Directory) {
            std::fs::create_dir_all(dest.join(path))?;
        }
    }

    let mut jobs = Vec::new();
    for (path, entry) in &manifest.files {
        if let Entry::File { downloads, .. } = entry {
            let target = dest.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            jobs.push(DownloadJob {
                url: downloads.raw.url.clone(),
                dest: target,
                sha1: Some(downloads.raw.sha1.clone()),
                size: downloads.raw.size,
            });
        }
    }
    run_jobs(client, &jobs, progress, VerifyMode::Fast, cancel).await?;

    for (path, entry) in &manifest.files {
        match entry {
            Entry::File { executable, .. } if *executable => {
                set_executable(&dest.join(path))?;
            }
            Entry::Link { target } => {
                make_link(&dest.join(path), target)?;
            }
            _ => {}
        }
    }

    let java = dest.join("bin").join(if cfg!(windows) { "java.exe" } else { "java" });
    if !java.exists() {
        anyhow::bail!("runtime downloaded but {} is missing", java.display());
    }
    Ok(java)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("marking {} executable", path.display()))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Recreate one of the manifest's symlinks. Targets are relative to the link.
///
/// Windows needs a privilege to make symlinks that a normal user hasn't got,
/// so there the link is a copy of whatever it pointed at.
fn make_link(link: &Path, target: &str) -> Result<()> {
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(link);

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("linking {} -> {target}", link.display()))
    }
    #[cfg(not(unix))]
    {
        let resolved = link
            .parent()
            .map(|p| p.join(target))
            .ok_or_else(|| anyhow!("link {} has no parent", link.display()))?;
        if resolved.is_dir() {
            return Ok(());
        }
        std::fs::copy(&resolved, link)
            .with_context(|| format!("copying {} -> {}", resolved.display(), link.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::major_of;

    #[test]
    fn major_from_runtime_version_names() {
        assert_eq!(major_of("25.0.1"), Some(25));
        assert_eq!(major_of("17.0.15"), Some(17));
        assert_eq!(major_of("16.0.1.9.1"), Some(16));
        // Mojang writes the Java 8 runtime this way.
        assert_eq!(major_of("8u202"), Some(8));
        assert_eq!(major_of("8u51-cacert462b08"), Some(8));
        assert_eq!(major_of(""), None);
    }
}
