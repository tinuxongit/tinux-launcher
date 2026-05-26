use crate::download::{install_version, ProgressEvent};
use crate::event::WorkerMsg;
use crate::java::JavaInstall;
use crate::launch::{self, LaunchOptions};
use crate::manifest::ManifestVersion;
use crate::paths::Paths;
use crate::version::VersionDetails;
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
    tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });

    match install_version(&client, &paths, &entry.id, &entry.url, &prog_tx).await {
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
    let (prog_tx, mut prog_rx) = mpsc::unbounded_channel::<ProgressEvent>();
    let app_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(ev) = prog_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::InstallProgress {
                done: ev.done,
                total: ev.total,
                what: ev.what,
            });
        }
    });

    let plan = match install_version(&client, &paths, &entry.id, &entry.url, &prog_tx).await {
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

    if let Some(req) = &details.java_version {
        if java.major < req.major_version {
            let _ = tx.send(WorkerMsg::LaunchFailed(format!(
                "Minecraft {} needs Java {}, but only Java {} is on PATH",
                version_id, req.major_version, java.major
            )));
            return;
        }
    }

    let (log_tx, mut log_rx) = mpsc::unbounded_channel::<String>();
    let app_tx = tx.clone();
    tokio::spawn(async move {
        while let Some(line) = log_rx.recv().await {
            let _ = app_tx.send(WorkerMsg::LaunchLog(line));
        }
    });

    let _ = tx.send(WorkerMsg::LaunchStarted(version_id.clone()));
    let res = launch::launch(&java, &paths, &plan, &details, &opts, log_tx).await;
    match res {
        Ok(code) => {
            let _ = tx.send(WorkerMsg::LaunchExited(code));
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::LaunchFailed(format!("{e:#}")));
        }
    }

}
