use crate::download::{install_version, ProgressEvent};
use crate::event::{InstallKind, WorkerMsg};
use crate::fabric;
use crate::java::{self, JavaInstall};
use crate::launch::{self, LaunchOptions};
use crate::manifest::{ManifestVersion, VersionKind, VersionManifest};
use crate::paths::Paths;
use crate::version::VersionDetails;
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedSender};

pub async fn do_install(
    client: reqwest::Client,
    paths: Paths,
    entry: ManifestVersion,
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

    let result = install_version(&client, &paths, &entry.id, &entry.url, &prog_tx).await;
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

    let result = install_version(&client, &paths, &entry.id, &entry.url, &prog_tx).await;
    drop(prog_tx);
    let _ = forwarder.await;
    let plan = match result {
        Ok(p) => p,
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
    } else if java.major == required_java {
        java
    } else if let Some(found) = java::detect_for_major(required_java) {
        found
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
        let _ = tx.send(WorkerMsg::LaunchFailed(format!(
            "Minecraft {} needs Java {}, but {found}. Install Java {} or put it on PATH.",
            version_id, required_java, required_java
        )));
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

    let result = install_version(&client, &paths, &fabric_id, "", &prog_tx).await;
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

pub async fn do_install_and_launch_fabric(
    client: reqwest::Client,
    paths: Paths,
    manifest: Arc<VersionManifest>,
    mc_version: String,
    loader_version: String,
    java: JavaInstall,
    opts: LaunchOptions,
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
    do_install_and_launch(client, paths, entry, java, opts, tx).await;
}

pub async fn do_verify_integrity(
    client: reqwest::Client,
    paths: Paths,
    entry: ManifestVersion,
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
    let result = install_version(&client, &paths, &entry.id, &entry.url, &prog_tx).await;
    drop(prog_tx);
    let _ = forwarder.await;
    match result {
        Ok(plan) => {
            let checked = plan.jobs.len();
            let _ = tx.send(WorkerMsg::VerifyDone {
                version: version_id,
                checked,
                repaired: 0,
                missing: 0,
            });
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::VerifyFailed(format!("{e:#}")));
        }
    }
}

fn required_java_major(details: &VersionDetails) -> u32 {
    details
        .java_version
        .as_ref()
        .map(|req| req.major_version)
        .unwrap_or(8)
}
