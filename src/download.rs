use crate::paths::{ensure_parent, Paths};
use crate::version::{self, Artifact, VersionDetails};
use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;

/// Shared cancel signal for an in-flight install. Set to true to make every
/// pending and in-progress download bail out with a "cancelled by user" error.
pub type CancelFlag = Arc<AtomicBool>;

pub fn new_cancel_flag() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

pub fn is_cancel_error(error: &str) -> bool {
    error.contains("cancelled by user")
}

/// How thoroughly existing files are checked before being trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    /// Existence + size check only. Used on the launch path so starting the
    /// game doesn't re-hash thousands of asset files every time.
    Fast,
    /// Full SHA1 re-hash of every file (explicit installs and Verify Integrity).
    Full,
}

/// What actually happened across a `run_jobs` pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct RunSummary {
    pub checked: usize,
    /// Files that didn't exist and were downloaded.
    pub missing: usize,
    /// Files that existed but failed verification and were re-downloaded.
    pub repaired: usize,
}

#[derive(Debug, Clone)]
pub struct ProgressEvent {
    pub done: u64,
    pub total: u64,
    pub what: String,
}

#[derive(Debug, Clone)]
pub struct DownloadJob {
    pub url: String,
    pub dest: PathBuf,
    pub sha1: Option<String>,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub version_id: String,
    pub jobs: Vec<DownloadJob>,
    pub resource_assets: Vec<ResourceAsset>,
    pub natives_jars: Vec<PathBuf>,
    pub classpath: Vec<PathBuf>,
    pub asset_index_id: String,
    pub asset_legacy_or_resources: AssetLayout,
    pub main_class: String,
    pub client_jar: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ResourceAsset {
    pub name: String,
    pub hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetLayout {
    Modern,
    Legacy,
    PreVirtual,
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
    #[serde(default)]
    map_to_resources: bool,
    objects: std::collections::BTreeMap<String, AssetObject>,
}

#[derive(Debug, Deserialize)]
struct AssetObject {
    hash: String,
    size: u64,
}

pub async fn install_version(
    client: &reqwest::Client,
    paths: &Paths,
    version_id: &str,
    version_url: &str,
    progress: &UnboundedSender<ProgressEvent>,
    mode: VerifyMode,
    cancel: &CancelFlag,
) -> Result<(InstallPlan, RunSummary)> {
    let plan = build_plan(client, paths, version_id, version_url).await?;
    let summary = run_jobs(client, &plan.jobs, progress, mode, cancel).await?;
    materialize_resource_assets(paths, &plan).await?;
    Ok((plan, summary))
}

async fn build_plan(
    client: &reqwest::Client,
    paths: &Paths,
    version_id: &str,
    version_url: &str,
) -> Result<InstallPlan> {
    let vj_path = paths.version_json(version_id);
    ensure_parent(&vj_path)?;
    let details = if vj_path.exists() {
        let bytes = fs::read(&vj_path).await?;
        serde_json::from_slice::<VersionDetails>(&bytes)
            .with_context(|| format!("parsing cached {}", vj_path.display()))?
    } else {
        let details = version::fetch_details(client, version_url).await?;
        fs::write(&vj_path, serde_json::to_vec_pretty(&details)?).await?;
        details
    };

    // 2. asset index
    let ai_path = paths.assets_indexes.join(format!("{}.json", details.asset_index.id));
    ensure_parent(&ai_path)?;
    let ai_bytes = if ai_path.exists() {
        fs::read(&ai_path).await?
    } else {
        let b = client
            .get(&details.asset_index.url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec();
        fs::write(&ai_path, &b).await?;
        b
    };
    let asset_index: AssetIndex = serde_json::from_slice(&ai_bytes)?;

    let asset_layout = match details.assets.as_str() {
        "legacy" => AssetLayout::Legacy,
        "pre-1.6" => AssetLayout::PreVirtual,
        _ => AssetLayout::Modern,
    };

    // 3. build job list
    let mut jobs: Vec<DownloadJob> = Vec::new();

    // client jar
    let client_jar = paths.version_jar(version_id);
    jobs.push(DownloadJob {
        url: details.downloads.client.url.clone(),
        dest: client_jar.clone(),
        sha1: Some(details.downloads.client.sha1.clone()),
        size: details.downloads.client.size,
    });

    let mut classpath: Vec<PathBuf> = Vec::new();
    let mut natives_jars: Vec<PathBuf> = Vec::new();

    for lib in &details.libraries {
        if !version::library_included(lib) {
            continue;
        }
        let Some(dl) = &lib.downloads else { continue };
        if let Some(artifact) = &dl.artifact {
            let dest = paths.library_path(&artifact.path);
            jobs.push(job_for_artifact(artifact, &dest));
            classpath.push(dest);
        }
        if let Some(classifier) = version::natives_classifier(lib) {
            if let Some(classifiers) = &dl.classifiers {
                if let Some(art) = classifiers.get(&classifier) {
                    let dest = paths.library_path(&art.path);
                    jobs.push(job_for_artifact(art, &dest));
                    natives_jars.push(dest);
                }
            }
        }
    }

    // assets
    let mut resource_assets = Vec::new();
    for (_name, obj) in &asset_index.objects {
        let dest = paths.asset_object(&obj.hash);
        let url = format!(
            "https://resources.download.minecraft.net/{}/{}",
            &obj.hash[..2],
            &obj.hash
        );
        jobs.push(DownloadJob {
            url,
            dest,
            sha1: Some(obj.hash.clone()),
            size: obj.size,
        });
    }
    if asset_index.map_to_resources {
        resource_assets.extend(asset_index.objects.iter().map(|(name, obj)| ResourceAsset {
            name: name.clone(),
            hash: obj.hash.clone(),
        }));
    }

    Ok(InstallPlan {
        version_id: details.id.clone(),
        jobs,
        resource_assets,
        natives_jars,
        classpath,
        asset_index_id: details.asset_index.id.clone(),
        asset_legacy_or_resources: asset_layout,
        main_class: details.main_class.clone(),
        client_jar,
    })
}

async fn materialize_resource_assets(paths: &Paths, plan: &InstallPlan) -> Result<()> {
    if plan.resource_assets.is_empty() {
        return Ok(());
    }
    let resources = paths.instances.join(&plan.version_id).join("resources");
    for asset in &plan.resource_assets {
        let src = paths.asset_object(&asset.hash);
        let dest = resources.join(&asset.name);
        if dest.exists() && sha1_of_file(&dest).await? == asset.hash {
            continue;
        }
        ensure_parent(&dest)?;
        fs::copy(&src, &dest)
            .await
            .with_context(|| format!("copying resource asset {}", asset.name))?;
    }
    Ok(())
}

fn job_for_artifact(a: &Artifact, dest: &Path) -> DownloadJob {
    DownloadJob {
        url: a.url.clone(),
        dest: dest.to_path_buf(),
        // Fabric's library entries don't include sha1; treat empty as unverified.
        sha1: if a.sha1.is_empty() { None } else { Some(a.sha1.clone()) },
        size: a.size,
    }
}

/// Outcome of checking/fetching a single job, for the run summary.
enum FileOutcome {
    AlreadyOk,
    FetchedMissing,
    Repaired,
}

pub async fn run_jobs(
    client: &reqwest::Client,
    jobs: &[DownloadJob],
    progress: &UnboundedSender<ProgressEvent>,
    mode: VerifyMode,
    cancel: &CancelFlag,
) -> Result<RunSummary> {
    let total: u64 = jobs.iter().map(|j| j.size).sum();
    let done = Arc::new(AtomicU64::new(0));
    let counter = Arc::new(AtomicU64::new(0));
    let missing = Arc::new(AtomicU64::new(0));
    let repaired = Arc::new(AtomicU64::new(0));
    let count_total = jobs.len() as u64;
    let sem = Arc::new(Semaphore::new(16));

    let mut futs = FuturesUnordered::new();
    for job in jobs.iter().cloned() {
        let permit_sem = sem.clone();
        let client = client.clone();
        let done = done.clone();
        let counter = counter.clone();
        let missing = missing.clone();
        let repaired = repaired.clone();
        let progress = progress.clone();
        let cancel = cancel.clone();
        futs.push(tokio::spawn(async move {
            let Ok(_permit) = permit_sem.acquire_owned().await else {
                anyhow::bail!("download pool closed");
            };
            let res = ensure_file(&client, &job, mode, &cancel).await;
            match &res {
                Ok(FileOutcome::FetchedMissing) => {
                    missing.fetch_add(1, Ordering::Relaxed);
                }
                Ok(FileOutcome::Repaired) => {
                    repaired.fetch_add(1, Ordering::Relaxed);
                }
                _ => {}
            }
            let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
            let d = done.fetch_add(job.size, Ordering::Relaxed) + job.size;
            let _ = progress.send(ProgressEvent {
                done: d,
                total,
                what: format!(
                    "{n}/{count_total}  {}",
                    job.dest
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default()
                ),
            });
            res.map(|_| ())
        }));
    }

    let mut first_err = None;
    while let Some(joined) = futs.next().await {
        let res = joined.map_err(anyhow::Error::from).and_then(|r| r);
        if let Err(e) = res {
            if first_err.is_none() {
                // Stop the still-queued jobs instead of letting them keep
                // downloading behind an error we've already reported.
                cancel.store(true, Ordering::Relaxed);
                first_err = Some(e);
            }
        }
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(RunSummary {
        checked: jobs.len(),
        missing: missing.load(Ordering::Relaxed) as usize,
        repaired: repaired.load(Ordering::Relaxed) as usize,
    })
}

async fn ensure_file(
    client: &reqwest::Client,
    job: &DownloadJob,
    mode: VerifyMode,
    cancel: &CancelFlag,
) -> Result<FileOutcome> {
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("cancelled by user");
    }
    let existed = job.dest.exists();
    if existed && file_ok(&job.dest, job, mode).await? {
        return Ok(FileOutcome::AlreadyOk);
    }
    ensure_parent(&job.dest)?;
    let tmp = job.dest.with_extension("part");
    let mut resp = client
        .get(&job.url)
        .send()
        .await
        .with_context(|| format!("GET {}", job.url))?
        .error_for_status()
        .with_context(|| format!("status for {}", job.url))?;
    let mut file = fs::File::create(&tmp).await?;
    let mut hasher = Sha1::new();
    while let Some(chunk) = resp.chunk().await? {
        if cancel.load(Ordering::Relaxed) {
            drop(file);
            let _ = fs::remove_file(&tmp).await;
            anyhow::bail!("cancelled by user");
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    if let Some(expected) = &job.sha1 {
        let got = hex::encode(hasher.finalize());
        if &got != expected {
            let _ = fs::remove_file(&tmp).await;
            anyhow::bail!("hash mismatch for {}: got {got}, want {expected}", job.url);
        }
    }
    fs::rename(&tmp, &job.dest).await?;
    Ok(if existed {
        FileOutcome::Repaired
    } else {
        FileOutcome::FetchedMissing
    })
}

/// Is an existing file trustworthy? Fast mode accepts any file whose size
/// matches the manifest (or any file at all when the size is unknown);
/// Full mode re-hashes the contents.
async fn file_ok(p: &Path, job: &DownloadJob, mode: VerifyMode) -> Result<bool> {
    let Some(expected) = job.sha1.as_deref() else {
        return Ok(true);
    };
    match mode {
        VerifyMode::Fast => {
            if job.size == 0 {
                return Ok(true);
            }
            let meta = fs::metadata(p).await?;
            Ok(meta.len() == job.size)
        }
        VerifyMode::Full => Ok(sha1_of_file(p).await? == expected),
    }
}

async fn sha1_of_file(p: &Path) -> Result<String> {
    let bytes = fs::read(p).await?;
    let mut h = Sha1::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}

pub async fn extract_natives(natives_jars: &[PathBuf], dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).await?;
    for jar in natives_jars {
        let jar = jar.clone();
        let dest = dest.to_path_buf();
        tokio::task::spawn_blocking(move || extract_one(&jar, &dest))
            .await??;
    }
    Ok(())
}

fn extract_one(jar: &Path, dest: &Path) -> Result<()> {
    let file = std::fs::File::open(jar)
        .with_context(|| format!("opening {}", jar.display()))?;
    let mut zip = zip::ZipArchive::new(file)?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if name.ends_with('/') || name.starts_with("META-INF/") {
            continue;
        }
        let lower = name.to_lowercase();
        let is_native = lower.ends_with(".dll")
            || lower.ends_with(".dylib")
            || lower.ends_with(".so")
            || lower.ends_with(".jnilib");
        if !is_native {
            continue;
        }
        let Some(fname) = std::path::Path::new(&name).file_name() else {
            continue;
        };
        let out = dest.join(fname);
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_with(dest: PathBuf, sha1: &str, size: u64) -> DownloadJob {
        DownloadJob {
            url: String::new(),
            dest,
            sha1: Some(sha1.to_string()),
            size,
        }
    }

    #[tokio::test]
    async fn fast_mode_trusts_size_full_mode_hashes() {
        let dir = std::env::temp_dir().join("tinux-launcher-test-file-ok");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.bin");
        std::fs::write(&p, b"hello").unwrap();

        // Hash is wrong but size matches: Fast trusts it, Full rejects it.
        let wrong_hash = job_with(p.clone(), "0000000000000000000000000000000000000000", 5);
        assert!(file_ok(&p, &wrong_hash, VerifyMode::Fast).await.unwrap());
        assert!(!file_ok(&p, &wrong_hash, VerifyMode::Full).await.unwrap());

        // Size mismatch: Fast rejects too.
        let wrong_size = job_with(p.clone(), "0000000000000000000000000000000000000000", 6);
        assert!(!file_ok(&p, &wrong_size, VerifyMode::Fast).await.unwrap());

        // Correct hash passes Full.
        let right_hash = job_with(p.clone(), "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d", 5);
        assert!(file_ok(&p, &right_hash, VerifyMode::Full).await.unwrap());
    }
}
