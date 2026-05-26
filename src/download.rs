use crate::paths::{ensure_parent, Paths};
use crate::version::{self, Artifact, VersionDetails};
use anyhow::{Context, Result};
use futures::stream::{FuturesUnordered, StreamExt};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Semaphore;

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
    pub natives_jars: Vec<PathBuf>,
    pub classpath: Vec<PathBuf>,
    pub asset_index_id: String,
    pub asset_legacy_or_resources: AssetLayout,
    pub main_class: String,
    pub client_jar: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetLayout {
    Modern,
    Legacy,
    PreVirtual,
}

#[derive(Debug, Deserialize)]
struct AssetIndex {
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
) -> Result<InstallPlan> {
    let plan = build_plan(client, paths, version_id, version_url).await?;
    run_jobs(client, &plan.jobs, progress).await?;
    Ok(plan)
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

    Ok(InstallPlan {
        version_id: details.id.clone(),
        jobs,
        natives_jars,
        classpath,
        asset_index_id: details.asset_index.id.clone(),
        asset_legacy_or_resources: asset_layout,
        main_class: details.main_class.clone(),
        client_jar,
    })
}

fn job_for_artifact(a: &Artifact, dest: &Path) -> DownloadJob {
    DownloadJob {
        url: a.url.clone(),
        dest: dest.to_path_buf(),
        sha1: Some(a.sha1.clone()),
        size: a.size,
    }
}

pub async fn run_jobs(
    client: &reqwest::Client,
    jobs: &[DownloadJob],
    progress: &UnboundedSender<ProgressEvent>,
) -> Result<()> {
    let total: u64 = jobs.iter().map(|j| j.size).sum();
    let done = Arc::new(AtomicU64::new(0));
    let counter = Arc::new(AtomicU64::new(0));
    let count_total = jobs.len() as u64;
    let sem = Arc::new(Semaphore::new(16));

    let mut futs = FuturesUnordered::new();
    for job in jobs.iter().cloned() {
        let permit_sem = sem.clone();
        let client = client.clone();
        let done = done.clone();
        let counter = counter.clone();
        let progress = progress.clone();
        futs.push(tokio::spawn(async move {
            let _permit = permit_sem.acquire_owned().await.unwrap();
            let res = ensure_file(&client, &job).await;
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
            res
        }));
    }

    while let Some(joined) = futs.next().await {
        joined??;
    }
    Ok(())
}

async fn ensure_file(client: &reqwest::Client, job: &DownloadJob) -> Result<()> {
    if file_ok(&job.dest, job.sha1.as_deref()).await? {
        return Ok(());
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
    Ok(())
}

async fn file_ok(p: &Path, expected_sha1: Option<&str>) -> Result<bool> {
    if !p.exists() {
        return Ok(false);
    }
    let Some(expected) = expected_sha1 else {
        return Ok(true);
    };
    let bytes = fs::read(p).await?;
    let mut h = Sha1::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()) == expected)
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
        let out = dest.join(std::path::Path::new(&name).file_name().unwrap());
        let mut f = std::fs::File::create(&out)?;
        std::io::copy(&mut entry, &mut f)?;
    }
    Ok(())
}
