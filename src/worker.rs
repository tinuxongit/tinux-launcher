use crate::download::{install_version, CancelFlag, ProgressEvent, VerifyMode};
use crate::event::{InstallKind, WorkerMsg};
use crate::fabric;
use crate::forge::{self, ForgeKind};
use crate::java::{self, JavaInstall};
use crate::launch::{self, LaunchOptions};
use crate::manifest::{ManifestVersion, VersionKind, VersionManifest};
use crate::paths::Paths;
use crate::version::VersionDetails;
use sha1::{Digest, Sha1};
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedSender};

pub async fn do_install(
    client: reqwest::Client,
    paths: Paths,
    entry: ManifestVersion,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let version_id = entry.id.clone();
    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                kind: InstallKind::Install,
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });

    // Explicit installs do a full hash pass so they double as a repair.
    let result = install_version(
        &client,
        &paths,
        &entry.id,
        &entry.url,
        &prog_tx,
        VerifyMode::Full,
        &cancel,
    )
    .await;
    drop(prog_tx);
    let _ = forwarder.await;
    match result {
        Ok(_) => {
            let _ = tx.send(WorkerMsg::InstallDone(version_id));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::InstallFailed {
                version: version_id,
                error: format!("{e:#}"),
            });
        }
    }
}

pub async fn do_install_and_launch(
    client: reqwest::Client,
    paths: Paths,
    entry: ManifestVersion,
    java: JavaInstall,
    opts: LaunchOptions,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let version_id = entry.id.clone();
    let kind = if paths.version_jar(&version_id).exists() {
        InstallKind::Verify
    } else {
        InstallKind::Install
    };

    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                kind,
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });

    // Fast verification on the launch path: trust existing files by size so
    // starting the game doesn't re-hash every asset. The Verify Integrity
    // button still does the full pass.
    let result = install_version(
        &client,
        &paths,
        &entry.id,
        &entry.url,
        &prog_tx,
        VerifyMode::Fast,
        &cancel,
    )
    .await;
    drop(prog_tx);
    let _ = forwarder.await;
    let plan = match result {
        Ok((p, _)) => p,
        Err(e) => {
            let _ = tx.send(WorkerMsg::InstallFailed {
                version: version_id,
                error: format!("{e:#}"),
            });
            return;
        }
    };
    let _ = tx.send(WorkerMsg::InstallDone(version_id.clone()));

    let details_path = paths.version_json(&version_id);
    let details: VersionDetails = match tokio::fs::read(&details_path).await {
        Ok(b) => match serde_json::from_slice(&b) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(WorkerMsg::LaunchFailed(format!(
                    "parsing cached {}: {e}",
                    details_path.display()
                )));
                return;
            }
        },
        Err(e) => {
            let _ = tx.send(WorkerMsg::LaunchFailed(format!(
                "reading cached {}: {e}",
                details_path.display()
            )));
            return;
        }
    };

    let required_java = required_java_major(&details);
    // User-configured override wins over auto-detection. We trust the user
    // picked something sensible (detect_at_path probes it to read the major).
    let launch_java = if let Some(p) = opts.java_override.as_ref() {
        match java::detect_at_path(p) {
            Some(j) => j,
            None => {
                let _ = tx.send(WorkerMsg::LaunchFailed(format!(
                    "Java override {} isn't a working `java` executable",
                    p.display()
                )));
                return;
            }
        }
    } else if let Some(found) = java::detect_for_version(required_java) {
        // The closest match, not merely the first that qualifies: the default
        // Java may be far newer than the version wants, and mod loaders break
        // on that.
        found
    } else if java::major_can_run(java.major, required_java) {
        java
    } else {
        let found = java::detect_all()
            .into_iter()
            .map(|j| format!("Java {} at {}", j.major, j.path.display()))
            .collect::<Vec<_>>()
            .join(", ");
        let found = if found.is_empty() {
            "no Java installations detected".to_string()
        } else {
            found
        };
        // Java 8 versions won't run on anything newer, so the message says
        // exactly 8 there and "or newer" for everything since 1.17.
        let wanted = if required_java <= 8 {
            format!("Java {required_java}")
        } else {
            format!("Java {required_java} or newer")
        };
        let _ = tx.send(WorkerMsg::JavaMissing {
            major: required_java,
            component: required_java_component(&details),
            detail: format!("Minecraft {version_id} needs {wanted}. Found {found}."),
        });
        return;
    };

    let (log_tx, mut log_rx) = mpsc::unbounded_channel::<String>();
    let app_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::LaunchLog(line));
        }
    });

    let _ = tx.send(WorkerMsg::LaunchStarted(version_id.clone()));
    let _ = tx.send(WorkerMsg::LaunchLog(format!(
        "Using Java {} at {}",
        launch_java.major,
        launch_java.path.display()
    )));
    let res = launch::launch(&launch_java, &paths, &plan, &details, &opts, log_tx).await;
    match res {
        Ok(code) => {
            let _ = tx.send(WorkerMsg::LaunchExited(code));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::LaunchFailed(format!("{e:#}")));
        }
    }

}

pub async fn do_install_fabric(
    client: reqwest::Client,
    paths: Paths,
    manifest: Arc<VersionManifest>,
    mc_version: String,
    loader_version: String,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let fabric_id = match fabric::prepare_fabric_version(
        &client,
        &paths,
        &manifest,
        &mc_version,
        &loader_version,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            let _ = tx.send(WorkerMsg::InstallFailed {
                version: format!("fabric/{mc_version}"),
                error: format!("{e:#}"),
            });
            return;
        }
    };

    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                kind: InstallKind::Install,
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });

    let result = install_version(
        &client,
        &paths,
        &fabric_id,
        "",
        &prog_tx,
        VerifyMode::Full,
        &cancel,
    )
    .await;
    drop(prog_tx);
    let _ = forwarder.await;
    match result {
        Ok(_) => {
            let _ = tx.send(WorkerMsg::InstallDone(fabric_id));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::InstallFailed {
                version: fabric_id,
                error: format!("{e:#}"),
            });
        }
    }
}

/// Install a Forge/NeoForge version (no launch). Runs the official installer
/// if needed, then the normal full-verify install pass.
#[allow(clippy::too_many_arguments)]
pub async fn do_install_forge(
    client: reqwest::Client,
    paths: Paths,
    manifest: Arc<VersionManifest>,
    kind: ForgeKind,
    mc_version: String,
    loader_version: String,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let progress_tx = tx.clone();
    let progress = move |what: String| {
        let _ = progress_tx.send(WorkerMsg::InstallProgress {
            kind: InstallKind::Install,
            done: 0,
            total: 0,
            what,
        });
    };
    let id = match forge::prepare_version(
        &client,
        &paths,
        &manifest,
        kind,
        &mc_version,
        &loader_version,
        None,
        &cancel,
        progress,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            let _ = tx.send(WorkerMsg::InstallFailed {
                version: format!("{}/{mc_version}", kind.label()),
                error: format!("{e:#}"),
            });
            return;
        }
    };

    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                kind: InstallKind::Install,
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });
    let result = install_version(
        &client,
        &paths,
        &id,
        "",
        &prog_tx,
        VerifyMode::Full,
        &cancel,
    )
    .await;
    drop(prog_tx);
    let _ = forwarder.await;
    match result {
        Ok(_) => {
            let _ = tx.send(WorkerMsg::InstallDone(id));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::InstallFailed {
                version: id,
                error: format!("{e:#}"),
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn do_install_and_launch_forge(
    client: reqwest::Client,
    paths: Paths,
    manifest: Arc<VersionManifest>,
    kind: ForgeKind,
    mc_version: String,
    loader_version: String,
    java: JavaInstall,
    opts: LaunchOptions,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let progress_tx = tx.clone();
    let progress = move |what: String| {
        let _ = progress_tx.send(WorkerMsg::InstallProgress {
            kind: InstallKind::Install,
            done: 0,
            total: 0,
            what,
        });
    };
    let id = match forge::prepare_version(
        &client,
        &paths,
        &manifest,
        kind,
        &mc_version,
        &loader_version,
        None,
        &cancel,
        progress,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            let _ = tx.send(WorkerMsg::LaunchFailed(format!(
                "{} setup failed: {e:#}",
                kind.label()
            )));
            return;
        }
    };
    let entry = ManifestVersion {
        id,
        kind: VersionKind::Release,
        url: String::new(),
        sha1: String::new(),
        release_time: String::new(),
    };
    do_install_and_launch(client, paths, entry, java, opts, cancel, tx).await;
}

#[allow(clippy::too_many_arguments)]
pub async fn do_install_and_launch_fabric(
    client: reqwest::Client,
    paths: Paths,
    manifest: Arc<VersionManifest>,
    mc_version: String,
    loader_version: String,
    java: JavaInstall,
    opts: LaunchOptions,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let fabric_id = match fabric::prepare_fabric_version(
        &client,
        &paths,
        &manifest,
        &mc_version,
        &loader_version,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            let _ = tx.send(WorkerMsg::LaunchFailed(format!("Fabric setup failed: {e:#}")));
            return;
        }
    };
    let entry = ManifestVersion {
        id: fabric_id,
        kind: VersionKind::Release,
        url: String::new(),
        sha1: String::new(),
        release_time: String::new(),
    };
    do_install_and_launch(client, paths, entry, java, opts, cancel, tx).await;
}

#[allow(clippy::too_many_arguments)]
pub async fn do_install_modpack(
    client: reqwest::Client,
    paths: Paths,
    manifest: Arc<VersionManifest>,
    browse_mc_version: String,
    project_id: String,
    name: String,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    match install_modpack_inner(
        &client,
        &paths,
        &manifest,
        &browse_mc_version,
        &project_id,
        &name,
        &cancel,
        &tx,
    )
    .await
    {
        Ok(instance) => {
            let _ = tx.send(WorkerMsg::ModpackInstallDone(instance));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::ModpackInstallFailed(format!("{e:#}")));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn install_modpack_inner(
    client: &reqwest::Client,
    paths: &Paths,
    manifest: &VersionManifest,
    browse_mc_version: &str,
    project_id: &str,
    name: &str,
    cancel: &CancelFlag,
    tx: &UnboundedSender<WorkerMsg>,
) -> anyhow::Result<crate::config::ModpackInstance> {
    let progress = |done: u64, total: u64, what: &str| {
        let _ = tx.send(WorkerMsg::ModpackInstallProgress {
            done,
            total,
            what: what.to_string(),
        });
    };

    // 1. Resolve and download the .mrpack archive (small — just an index +
    //    overrides; the mods themselves are fetched by URL afterwards).
    progress(0, 0, "Resolving modpack");
    let file = crate::modrinth::fetch_modpack_file(client, project_id, browse_mc_version).await?;
    progress(0, 0, "Downloading modpack");
    let bytes = client
        .get(&file.url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?
        .to_vec();
    if let Some(expected) = file.sha1.as_deref() {
        let got = hex::encode(Sha1::digest(&bytes));
        if got != expected {
            anyhow::bail!(
                "hash mismatch for modpack archive {}: got {got}, want {expected}",
                file.filename
            );
        }
    }

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        anyhow::bail!("cancelled by user");
    }

    // 2. Parse + work out which loader the pack needs.
    let index = crate::modpack::parse_mrpack(&bytes)?;
    let req = crate::modpack::loader_requirement(&index)?;
    let mc = req.mc_version;
    let loader_version = req.loader_version;
    let id = crate::modpack::instance_id(project_id, &mc);
    let display_name = if name.trim().is_empty() {
        index.name.clone()
    } else {
        name.to_string()
    };

    // 3. Build + install the loader runtime under this modpack's own id.
    progress(
        0,
        0,
        &format!("Installing {} for {mc}", req.loader.as_str()),
    );
    match req.loader {
        crate::modpack::PackLoader::Fabric => {
            crate::fabric::prepare_fabric_version_as(
                client,
                paths,
                manifest,
                &mc,
                &loader_version,
                Some(id.as_str()),
            )
            .await?;
        }
        crate::modpack::PackLoader::Forge | crate::modpack::PackLoader::NeoForge => {
            let kind = if req.loader == crate::modpack::PackLoader::Forge {
                ForgeKind::Forge
            } else {
                ForgeKind::NeoForge
            };
            let progress_tx = tx.clone();
            let installer_progress = move |what: String| {
                let _ = progress_tx.send(WorkerMsg::ModpackInstallProgress {
                    done: 0,
                    total: 0,
                    what,
                });
            };
            forge::prepare_version(
                client,
                paths,
                manifest,
                kind,
                &mc,
                &loader_version,
                Some(id.as_str()),
                cancel,
                installer_progress,
            )
            .await?;
        }
    }
    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::ModpackInstallProgress {
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });
    let runtime = install_version(
        client,
        paths,
        &id,
        "",
        &prog_tx,
        VerifyMode::Fast,
        cancel,
    )
    .await;
    drop(prog_tx);
    let _ = forwarder.await;
    runtime?;

    // 4. Download the modpack's files into its instance dir.
    let instance_dir = paths.instances.join(&id);
    let progress_tx = tx.clone();
    crate::modpack::install_files(client, &index, &instance_dir, cancel, move |done, total, what| {
        let _ = progress_tx.send(WorkerMsg::ModpackInstallProgress { done, total, what });
    })
    .await?;

    // 5. Apply overrides (sync ZIP IO — off the async runtime).
    progress(0, 0, "Applying overrides");
    let dir = instance_dir.clone();
    tokio::task::spawn_blocking(move || crate::modpack::extract_overrides(&bytes, &dir))
        .await
        .map_err(|e| anyhow::anyhow!("override extraction panicked: {e}"))??;

    // 6. Register the instance so it persists and becomes launchable.
    let instance = crate::config::ModpackInstance {
        id,
        name: display_name,
        mc_version: mc,
        modpack_version: file.version_number,
        project_id: project_id.to_string(),
        loader: req.loader.as_str().to_string(),
    };
    crate::config::add_modpack(instance.clone());
    Ok(instance)
}

pub async fn do_verify_integrity(
    client: reqwest::Client,
    paths: Paths,
    entry: ManifestVersion,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let version_id = entry.id.clone();
    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                kind: InstallKind::Verify,
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });
    // install_version is idempotent: it re-downloads anything whose sha1
    // doesn't match, so calling it on an already-installed version is the
    // same operation as "verify integrity".
    let result = install_version(
        &client,
        &paths,
        &entry.id,
        &entry.url,
        &prog_tx,
        VerifyMode::Full,
        &cancel,
    )
    .await;
    drop(prog_tx);
    let _ = forwarder.await;
    match result {
        Ok((_, summary)) => {
            let _ = tx.send(WorkerMsg::VerifyDone {
                version: version_id,
                checked: summary.checked,
                repaired: summary.repaired,
                missing: summary.missing,
            });
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::VerifyFailed(format!("{e:#}")));
        }
    }
}

/// Mojang's name for the runtime this version wants, e.g.
/// `java-runtime-epsilon`. Older version JSONs leave it empty.
fn required_java_component(details: &VersionDetails) -> Option<String> {
    details
        .java_version
        .as_ref()
        .map(|req| req.component.clone())
        .filter(|c| !c.is_empty())
}

fn required_java_major(details: &VersionDetails) -> u32 {
    details
        .java_version
        .as_ref()
        .map(|req| req.major_version)
        .unwrap_or(8)
}

/// Fetch the Java a version asked for from Mojang and report where it landed.
///
/// Runs on the same download machinery as everything else, so it hashes every
/// file, reports progress and can be cancelled.
pub async fn do_download_java(
    client: reqwest::Client,
    paths: Paths,
    major: u32,
    component: Option<String>,
    cancel: CancelFlag,
    tx: UnboundedSender<WorkerMsg>,
) {
    let choice = match crate::runtime::find(&client, major, component.as_deref()).await {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(WorkerMsg::JavaDownloadFailed(format!("{e:#}")));
            return;
        }
    };
    let _ = tx.send(WorkerMsg::JavaDownloadStarted {
        major,
        version: choice.version.clone(),
    });

    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::JavaDownloadProgress {
                done: ev.done,
                total: ev.total,
            });
        }
    });

    let result =
        crate::runtime::install(&client, &paths.runtimes, &choice, &prog_tx, &cancel).await;
    drop(prog_tx);
    let _ = forwarder.await;

    match result {
        Ok(path) => {
            let _ = tx.send(WorkerMsg::JavaDownloadDone {
                version: choice.version,
                path,
            });
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::JavaDownloadFailed(format!("{e:#}")));
        }
    }
}
