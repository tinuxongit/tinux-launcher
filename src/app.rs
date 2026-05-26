use crate::auth::Account;
use crate::event::{Hit, Tab, WorkerMsg};
use crate::java::{self, JavaInstall};
use crate::manifest::{self, ManifestVersion, VersionKind, VersionManifest};
use crate::paths::Paths;
use ratatui::layout::Rect;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

const LOG_CAPACITY: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionFilter {
    Releases,
    Snapshots,
    Old,
}

#[derive(Debug)]
pub struct InstallState {
    pub done: u64,
    pub total: u64,
    pub what: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchState {
    Idle,
    Running,
    JustExited(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    None,
    OfflineName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountMode {
    Offline,
    Online,
}

pub struct App {
    pub running: bool,
    pub tab: Tab,
    pub paths: Paths,
    pub client: reqwest::Client,

    pub manifest: Option<Arc<VersionManifest>>,
    pub manifest_error: Option<String>,

    pub filter: VersionFilter,
    pub list_offset: usize,
    pub selected_version: Option<String>,

    pub hover: Option<Hit>,
    pub click_regions: Vec<(Rect, Hit)>,

    pub account: Option<Account>,
    pub account_mode: AccountMode,
    pub offline_name: String,
    pub focus: Focus,
    pub auth_in_progress: bool,
    pub auth_error: Option<String>,

    pub install: Option<InstallState>,
    pub launch_state: LaunchState,
    pub launch_error: Option<String>,

    pub logs: VecDeque<String>,
    pub log_offset: usize,

    pub java: Option<JavaInstall>,
    pub status_message: String,

    pub needs_clear: bool,

    pub selected_log_range: Option<(usize, usize)>,

    pub worker_tx: UnboundedSender<WorkerMsg>,

    pub last_size: Rect,
}

impl App {
    pub fn new(paths: Paths, worker_tx: UnboundedSender<WorkerMsg>) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("tinux-launcher/0.1")
            .pool_max_idle_per_host(16)
            .build()
            .expect("reqwest client");
        let java = java::detect_default();
        let status_message = match &java {
            Some(j) => format!("Java {} detected at {}", j.major, j.path.display()),
            None => "Java not found on PATH".into(),
        };
        Self {
            running: true,
            tab: Tab::Play,
            paths,
            client,
            manifest: None,
            manifest_error: None,
            filter: VersionFilter::Releases,
            list_offset: 0,
            selected_version: None,
            hover: None,
            click_regions: Vec::with_capacity(64),
            account: None,
            account_mode: AccountMode::Offline,
            offline_name: "Steve".into(),
            focus: Focus::None,
            auth_in_progress: false,
            auth_error: None,
            install: None,
            launch_state: LaunchState::Idle,
            launch_error: None,
            logs: VecDeque::with_capacity(LOG_CAPACITY),
            log_offset: 0,
            java,
            status_message,
            needs_clear: false,
            selected_log_range: None,
            worker_tx,
            last_size: Rect::default(),
        }
    }

    pub fn push_log(&mut self, line: String) {
        if self.logs.len() == LOG_CAPACITY {
            self.logs.pop_front();
            if let Some((a, b)) = self.selected_log_range {
                if a == 0 || b == 0 {
                    self.selected_log_range = None;
                } else {
                    self.selected_log_range = Some((a - 1, b - 1));
                }
            }
        }
        self.logs.push_back(sanitize_log(&line));
    }

    pub fn visible_versions(&self) -> Vec<&ManifestVersion> {
        let Some(m) = &self.manifest else { return Vec::new() };
        m.versions
            .iter()
            .filter(|v| match self.filter {
                VersionFilter::Releases => v.kind == VersionKind::Release,
                VersionFilter::Snapshots => v.kind == VersionKind::Snapshot,
                VersionFilter::Old => matches!(v.kind, VersionKind::OldBeta | VersionKind::OldAlpha),
            })
            .collect()
    }

    pub fn selected_manifest_entry(&self) -> Option<ManifestVersion> {
        let id = self.selected_version.as_ref()?;
        let m = self.manifest.as_ref()?;
        m.versions.iter().find(|v| &v.id == id).cloned()
    }

    pub fn ensure_default_selection(&mut self) {
        if self.selected_version.is_some() {
            return;
        }
        if let Some(m) = &self.manifest {
            self.selected_version = Some(m.latest.release.clone());
        }
    }

    pub fn handle_worker(&mut self, msg: WorkerMsg) {
        match msg {
            WorkerMsg::ManifestLoaded(m) => {
                self.manifest = Some(m);
                self.manifest_error = None;
                self.ensure_default_selection();
                self.status_message = "Version manifest loaded".into();
            }
            WorkerMsg::ManifestFailed(e) => {
                self.manifest_error = Some(e.clone());
                self.status_message = format!("Manifest error: {e}");
            }
            WorkerMsg::AuthStarted => {
                self.auth_in_progress = true;
                self.auth_error = None;
                self.status_message = "Opened browser for Microsoft sign-in...".into();
            }
            WorkerMsg::AuthSucceeded(a) => {
                self.auth_in_progress = false;
                self.status_message = format!("Signed in as {}", a.username);
                self.account = Some(a);
                self.account_mode = AccountMode::Online;
            }
            WorkerMsg::AuthFailed(e) => {
                self.auth_in_progress = false;
                self.auth_error = Some(e.clone());
                self.status_message = format!("Sign-in failed: {e}");
            }
            WorkerMsg::InstallProgress { done, total, what } => {
                self.install = Some(InstallState { done, total, what });
            }
            WorkerMsg::InstallDone(v) => {
                self.install = None;
                self.status_message = format!("Installed {v}");
            }
            WorkerMsg::InstallFailed { version, error } => {
                self.install = None;
                self.status_message = format!("Install failed for {version}: {error}");
            }
            WorkerMsg::LaunchStarted(v) => {
                self.launch_state = LaunchState::Running;
                self.status_message = format!("Launched {v}");
                self.needs_clear = true;
            }
            WorkerMsg::LaunchLog(line) => {
                self.push_log(line);
            }
            WorkerMsg::LaunchExited(code) => {
                self.launch_state = LaunchState::JustExited(code);
                self.status_message = format!("Minecraft exited with code {code}");
                self.needs_clear = true;
            }
            WorkerMsg::LaunchFailed(e) => {
                self.launch_state = LaunchState::Idle;
                self.launch_error = Some(e.clone());
                self.status_message = format!("Launch failed: {e}");
                self.needs_clear = true;
            }
        }
    }
}

fn sanitize_log(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut iter = s.chars();
    while let Some(c) = iter.next() {
        if c == '\x1B' {
            match iter.next() {
                Some('[') => {
                    for c2 in iter.by_ref() {
                        let v = c2 as u32;
                        if (0x40..=0x7E).contains(&v) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c2) = iter.next() {
                        if c2 == '\x07' {
                            break;
                        }
                        if c2 == '\x1B' {
                            iter.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if c == '\t' {
            out.push_str("    ");
            continue;
        }
        let v = c as u32;
        if v < 0x20 || c == '\x7F' {
            continue;
        }
        out.push(c);
    }
    out
}

pub fn spawn_manifest_fetch(
    client: reqwest::Client,
    tx: UnboundedSender<WorkerMsg>,
) {
    tokio::spawn(async move {
        match manifest::fetch(&client).await {
            Ok(m) => {
                let _ = tx.send(WorkerMsg::ManifestLoaded(Arc::new(m)));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::ManifestFailed(format!("{e:#}")));
            }
        }
    });
}
