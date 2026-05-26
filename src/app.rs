use crate::auth::Account;
use crate::event::{Hit, InstallKind, Tab, WorkerMsg};
use crate::java::{self, JavaInstall};
use crate::manifest::{self, ManifestVersion, VersionKind, VersionManifest};
use crate::meta::InstanceMeta;
use crate::modrinth::{Category, SearchHit};
use crate::news::{self, Article, NewsEntry};
use crate::paths::Paths;
use crate::skin::{SkinPreview, SkinView};
use crate::update::{self, UpdateInfo};
use ratatui::layout::Rect;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

const LOG_CAPACITY: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionFilter {
    Releases,
    Snapshots,
    Old,
    Modded,
}

impl VersionFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            VersionFilter::Releases => "releases",
            VersionFilter::Snapshots => "snapshots",
            VersionFilter::Old => "old",
            VersionFilter::Modded => "modded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "releases" => VersionFilter::Releases,
            "snapshots" => VersionFilter::Snapshots,
            "old" => VersionFilter::Old,
            "modded" => VersionFilter::Modded,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModLoader {
    Fabric,
}

impl ModLoader {
    pub fn label(self) -> &'static str {
        match self {
            ModLoader::Fabric => "Fabric",
        }
    }

    pub fn modrinth_key(self) -> &'static str {
        match self {
            ModLoader::Fabric => "fabric",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Mods,
    Shaders,
    ResourcePacks,
}

impl ContentKind {
    pub const ALL: [ContentKind; 3] = [
        ContentKind::Mods,
        ContentKind::Shaders,
        ContentKind::ResourcePacks,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ContentKind::Mods => "Mods",
            ContentKind::Shaders => "Shaders",
            ContentKind::ResourcePacks => "Texture Packs",
        }
    }

    pub fn project_type(self) -> &'static str {
        match self {
            ContentKind::Mods => "mod",
            ContentKind::Shaders => "shader",
            ContentKind::ResourcePacks => "resourcepack",
        }
    }

    pub fn folder(self) -> &'static str {
        match self {
            ContentKind::Mods => "mods",
            ContentKind::Shaders => "shaderpacks",
            ContentKind::ResourcePacks => "resourcepacks",
        }
    }

    /// Whether the Modrinth version-list query should filter by loader.
    pub fn uses_loader(self) -> bool {
        matches!(self, ContentKind::Mods)
    }
}

#[derive(Debug, Clone)]
pub struct InfoPopup {
    pub title: String,
    pub body: String,
}

impl InfoPopup {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self { title: title.into(), body: body.into() }
    }
}

#[derive(Debug, Clone)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate(String),
    Outdated(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        done: u64,
        total: u64,
    },
    Ready {
        info: UpdateInfo,
        new_exe: PathBuf,
    },
    Failed(String),
}

#[derive(Debug)]
pub struct InstallState {
    pub kind: InstallKind,
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
    SkinUrl,
    ModSearch,
    JavaPath,
    JavaPathForVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinModel {
    Classic,
    Slim,
}

impl SkinModel {
    pub fn as_str(self) -> &'static str {
        match self {
            SkinModel::Classic => "classic",
            SkinModel::Slim => "slim",
        }
    }
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

    pub news: Vec<NewsEntry>,
    pub news_offset: usize,
    pub viewing_news: Option<usize>,
    pub article: Option<Article>,
    pub article_loading: bool,
    pub article_offset: u16,
    pub news_split_top: Option<u16>,
    pub dragging_split: bool,
    pub dragging_scrollbar: Option<(Hit, Rect)>,
    pub play_inner: Rect,

    pub skin_preview: Option<SkinPreview>,
    pub skin_pending_preview: Option<SkinPreview>,
    pub skin_pending_loading: bool,
    pub skin_url_input: String,
    pub skin_model: SkinModel,
    pub skin_view: SkinView,
    pub skin_busy: bool,
    pub skin_error: Option<String>,

    pub logs: VecDeque<String>,
    pub log_offset: usize,

    pub java: Option<JavaInstall>,
    pub status_message: String,

    pub needs_clear: bool,

    pub selected_log_range: Option<(usize, usize)>,

    pub worker_tx: UnboundedSender<WorkerMsg>,

    pub last_size: Rect,

    pub update_status: UpdateStatus,
    pub update_modal_dismissed: bool,

    pub loader: ModLoader,
    pub fabric_loaders: Vec<String>,
    pub fabric_mc_versions: Vec<String>,

    pub mod_browser_open: bool,
    pub browser_kind: ContentKind,
    pub mod_search_query: String,
    pub mod_search_results: Vec<SearchHit>,
    pub mod_search_loading: bool,
    pub mod_search_error: Option<String>,
    pub mod_search_offset: usize,
    pub mod_search_api_offset: u32,
    pub mod_search_total: u32,
    pub mod_search_request_id: u64,
    pub mod_installing: Option<String>,
    pub installed_mods: Vec<String>,
    pub info_popup: Option<InfoPopup>,
    pub categories: Vec<Category>,
    pub selected_categories: Vec<String>,
    pub filters_popup_open: bool,
    pub filters_scroll: u16,
    pub installed_meta: InstanceMeta,

    pub max_ram_mb: u32,
    pub java_path_input: String,
    pub java_path_for_version_input: String,
    pub java_path_for_version_id: Option<String>,
    pub java_path_per_version: std::collections::HashMap<String, String>,

    pub integrity_in_progress: bool,
    pub installed_filter_only: bool,
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
        let cfg_opt = crate::config::path().map(|p| crate::config::Config::load(&p));
        let saved_offline = cfg_opt
            .as_ref()
            .and_then(|c| c.offline_name.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Steve".to_string());
        let saved_skin_url = cfg_opt
            .as_ref()
            .and_then(|c| c.offline_skin_url.clone())
            .unwrap_or_default();
        let saved_version = cfg_opt
            .as_ref()
            .and_then(|c| c.last_played_version.clone())
            .filter(|s| !s.trim().is_empty());
        let saved_filter = cfg_opt
            .as_ref()
            .and_then(|c| c.last_filter.as_deref().and_then(VersionFilter::parse))
            .unwrap_or(VersionFilter::Releases);
        let saved_max_ram = cfg_opt
            .as_ref()
            .and_then(|c| c.max_ram_mb)
            .unwrap_or(2048);
        let saved_java_path = cfg_opt
            .as_ref()
            .and_then(|c| c.java_path.clone())
            .unwrap_or_default();
        let saved_java_per_version = cfg_opt
            .as_ref()
            .map(|c| c.java_path_per_version.clone())
            .unwrap_or_default();
        Self {
            running: true,
            tab: Tab::Play,
            paths,
            client,
            manifest: None,
            manifest_error: None,
            filter: saved_filter,
            list_offset: 0,
            selected_version: saved_version,
            hover: None,
            click_regions: Vec::with_capacity(64),
            account: None,
            account_mode: AccountMode::Offline,
            offline_name: saved_offline,
            focus: Focus::None,
            auth_in_progress: false,
            auth_error: None,
            install: None,
            launch_state: LaunchState::Idle,
            launch_error: None,
            news: Vec::new(),
            news_offset: 0,
            viewing_news: None,
            article: None,
            article_loading: false,
            article_offset: 0,
            news_split_top: None,
            dragging_split: false,
            dragging_scrollbar: None,
            play_inner: Rect::default(),
            skin_preview: None,
            skin_pending_preview: None,
            skin_pending_loading: false,
            skin_url_input: saved_skin_url,
            skin_model: SkinModel::Classic,
            skin_view: SkinView::Front,
            skin_busy: false,
            skin_error: None,
            logs: VecDeque::with_capacity(LOG_CAPACITY),
            log_offset: 0,
            java,
            status_message,
            needs_clear: false,
            selected_log_range: None,
            worker_tx,
            last_size: Rect::default(),
            update_status: UpdateStatus::Idle,
            update_modal_dismissed: false,
            loader: ModLoader::Fabric,
            fabric_loaders: Vec::new(),
            fabric_mc_versions: Vec::new(),
            mod_browser_open: false,
            browser_kind: ContentKind::Mods,
            mod_search_query: String::new(),
            mod_search_results: Vec::new(),
            mod_search_loading: false,
            mod_search_error: None,
            mod_search_offset: 0,
            mod_search_api_offset: 0,
            mod_search_total: 0,
            mod_search_request_id: 0,
            mod_installing: None,
            installed_mods: Vec::new(),
            info_popup: None,
            categories: Vec::new(),
            selected_categories: Vec::new(),
            filters_popup_open: false,
            filters_scroll: 0,
            installed_meta: InstanceMeta::default(),
            max_ram_mb: saved_max_ram,
            java_path_input: saved_java_path,
            java_path_for_version_input: String::new(),
            java_path_for_version_id: None,
            java_path_per_version: saved_java_per_version,
            integrity_in_progress: false,
            installed_filter_only: false,
        }
    }

    pub fn java_path_override_for(&self, version_id: &str) -> Option<&str> {
        if let Some(p) = self.java_path_per_version.get(version_id) {
            if !p.is_empty() {
                return Some(p.as_str());
            }
        }
        if !self.java_path_input.is_empty() {
            return Some(self.java_path_input.as_str());
        }
        None
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
        match self.filter {
            VersionFilter::Releases => m
                .versions
                .iter()
                .filter(|v| v.kind == VersionKind::Release)
                .collect(),
            VersionFilter::Snapshots => m
                .versions
                .iter()
                .filter(|v| v.kind == VersionKind::Snapshot)
                .collect(),
            VersionFilter::Old => m
                .versions
                .iter()
                .filter(|v| matches!(v.kind, VersionKind::OldBeta | VersionKind::OldAlpha))
                .collect(),
            VersionFilter::Modded => {
                if self.fabric_mc_versions.is_empty() {
                    m.versions
                        .iter()
                        .filter(|v| v.kind == VersionKind::Release)
                        .collect()
                } else {
                    m.versions
                        .iter()
                        .filter(|v| self.fabric_mc_versions.iter().any(|s| s == &v.id))
                        .collect()
                }
            }
        }
    }

    pub fn latest_stable_fabric_loader(&self) -> Option<&str> {
        self.fabric_loaders.first().map(|s| s.as_str())
    }

    pub fn selected_modded_id(&self) -> Option<String> {
        self.modded_id_for(self.selected_version.as_ref()?)
    }

    pub fn selected_modded_installed(&self) -> bool {
        let Some(id) = self.selected_modded_id() else { return false };
        self.paths.version_json(&id).exists() && self.paths.version_jar(&id).exists()
    }

    pub fn current_content_dir(&self, kind: ContentKind) -> Option<PathBuf> {
        let id = self.selected_modded_id()?;
        Some(self.paths.instances.join(&id).join(kind.folder()))
    }

    pub fn visible_categories(&self) -> Vec<&Category> {
        let kind_key = self.browser_kind.project_type();
        self.categories
            .iter()
            .filter(|c| c.project_type == kind_key)
            .collect()
    }

    pub fn toggle_category(&mut self, name: &str) {
        if let Some(pos) = self.selected_categories.iter().position(|c| c == name) {
            self.selected_categories.remove(pos);
        } else {
            self.selected_categories.push(name.to_string());
        }
    }

    pub fn shaders_available(&self) -> bool {
        // The shaderpacks/ folder existing is the canonical signal (Iris/OptiFine
        // create it on first launch). As a fallback, detect a shader-loader mod
        // sitting in the mods/ folder — that's enough to unlock the tab even
        // before the user has run the game once with Iris installed.
        if self
            .current_content_dir(ContentKind::Shaders)
            .map(|p| p.exists())
            .unwrap_or(false)
        {
            return true;
        }
        let Some(mods_dir) = self.current_content_dir(ContentKind::Mods) else {
            return false;
        };
        let Ok(rd) = std::fs::read_dir(&mods_dir) else {
            return false;
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.ends_with(".jar") {
                continue;
            }
            // Match common shader-loader filenames. Iris ships as "iris-mc...",
            // OptiFine as "OptiFine_*", Oculus as "oculus-*", Canvas as "canvas-*".
            if name.starts_with("iris-")
                || name.starts_with("oculus-")
                || name.contains("optifine")
                || name.contains("optifabric")
                || name.starts_with("canvas-")
            {
                return true;
            }
        }
        false
    }

    pub fn current_instance_dir(&self) -> Option<PathBuf> {
        let id = self.selected_modded_id()?;
        Some(self.paths.instances.join(&id))
    }

    pub fn reload_meta(&mut self) {
        self.installed_meta = match self.current_instance_dir() {
            Some(d) => InstanceMeta::load(&d),
            None => InstanceMeta::default(),
        };
    }

    pub fn save_meta(&self) {
        if let Some(d) = self.current_instance_dir() {
            let _ = std::fs::create_dir_all(&d);
            self.installed_meta.save(&d);
        }
    }

    pub fn is_project_installed(&self, project_id: &str) -> bool {
        self.installed_meta.is_installed(self.browser_kind, project_id)
    }

    pub fn refresh_installed_mods(&mut self) {
        let Some(dir) = self.current_content_dir(self.browser_kind) else {
            self.installed_mods.clear();
            return;
        };
        let valid_ext: &[&str] = match self.browser_kind {
            ContentKind::Mods => &[".jar"],
            ContentKind::Shaders => &[".zip", ".jar"],
            ContentKind::ResourcePacks => &[".zip"],
        };
        let mut out: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let lower = name.to_ascii_lowercase();
                if valid_ext.iter().any(|ext| lower.ends_with(ext)) {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        out.sort();
        self.installed_mods = out;
    }

    pub fn selected_manifest_entry(&self) -> Option<ManifestVersion> {
        let id = self.selected_version.as_ref()?;
        let m = self.manifest.as_ref()?;
        m.versions.iter().find(|v| &v.id == id).cloned()
    }

    pub fn selected_is_installed(&self) -> bool {
        let Some(id) = &self.selected_version else {
            return false;
        };
        self.is_installed(id)
    }

    pub fn is_installed(&self, id: &str) -> bool {
        self.paths.version_json(id).exists() && self.paths.version_jar(id).exists()
    }

    pub fn modded_id_for(&self, mc_id: &str) -> Option<String> {
        // Same construction as selected_modded_id(); kept as a separate entry
        // point so callers can build the fabric id for an arbitrary mc id
        // (e.g. for the installed-check on every row of the modded list).
        let loader = self.latest_stable_fabric_loader()?;
        Some(format!("fabric-loader-{loader}-{mc_id}"))
    }

    pub fn install_in_progress(&self) -> bool {
        self.install.is_some()
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
                let uuid = a.uuid.clone();
                self.account = Some(a);
                self.account_mode = AccountMode::Online;
                spawn_skin_preview(self.client.clone(), self.worker_tx.clone(), uuid);
            }
            WorkerMsg::AuthFailed(e) => {
                self.auth_in_progress = false;
                self.auth_error = Some(e.clone());
                self.status_message = format!("Sign-in failed: {e}");
            }
            WorkerMsg::InstallProgress { kind, done, total, what } => {
                self.install = Some(InstallState { kind, done, total, what });
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
            WorkerMsg::NewsLoaded(entries) => {
                self.news = entries;
            }
            WorkerMsg::NewsFailed(e) => {
                tracing::warn!("news fetch failed: {e}");
            }
            WorkerMsg::ArticleLoaded { index, article } => {
                if self.viewing_news == Some(index) {
                    self.article = Some(article);
                    self.article_loading = false;
                    self.article_offset = 0;
                }
            }
            WorkerMsg::ArticleFailed { index, error } => {
                if self.viewing_news == Some(index) {
                    self.article_loading = false;
                    self.status_message = format!("Couldn't load article: {error}");
                    self.viewing_news = None;
                }
            }
            WorkerMsg::SkinPreviewLoaded(preview) => {
                self.skin_preview = Some(preview);
            }
            WorkerMsg::SkinPendingLoaded(preview) => {
                self.skin_pending_preview = Some(preview);
                self.skin_pending_loading = false;
                self.skin_error = None;
                self.status_message = "Skin previewed — Apply to upload".into();
            }
            WorkerMsg::SkinPendingFailed(e) => {
                self.skin_pending_loading = false;
                self.skin_error = Some(e.clone());
                self.status_message = format!("Preview failed: {e}");
            }
            WorkerMsg::SkinApplied => {
                self.skin_busy = false;
                self.skin_error = None;
                self.skin_url_input.clear();
                self.skin_pending_preview = None;
                self.status_message = "Skin updated".into();
            }
            WorkerMsg::SkinFailed(e) => {
                self.skin_busy = false;
                self.skin_error = Some(e.clone());
                self.status_message = format!("Skin change failed: {e}");
            }
            WorkerMsg::UpdateCheckStarted => {
                self.update_status = UpdateStatus::Checking;
                self.status_message = "Checking for updates...".into();
            }
            WorkerMsg::UpdateCheckResult(info) => {
                if info.up_to_date {
                    self.status_message = format!("You're on the latest version ({}).", info.current);
                    self.update_status = UpdateStatus::UpToDate(info.current);
                } else {
                    self.status_message =
                        format!("Update available: {} → {}", info.current, info.latest);
                    // Kick off the background download immediately.
                    if let Some(asset) = info.asset.clone() {
                        update::spawn_download(
                            self.client.clone(),
                            asset,
                            self.paths.cache.clone(),
                            self.worker_tx.clone(),
                        );
                        self.update_status = UpdateStatus::Downloading {
                            info,
                            done: 0,
                            total: 0,
                        };
                    } else {
                        self.update_status = UpdateStatus::Outdated(info);
                    }
                }
            }
            WorkerMsg::UpdateCheckFailed(e) => {
                self.status_message = format!("Update check failed: {e}");
                self.update_status = UpdateStatus::Failed(e);
            }
            WorkerMsg::UpdateDownloadStarted => {
                // The status was already set above when we kicked off the download.
            }
            WorkerMsg::UpdateDownloadProgress { done, total } => {
                if let UpdateStatus::Downloading { info, .. } = &self.update_status {
                    let info = info.clone();
                    self.update_status = UpdateStatus::Downloading { info, done, total };
                }
            }
            WorkerMsg::UpdateDownloaded(path) => {
                if let UpdateStatus::Downloading { info, .. } = &self.update_status {
                    let info = info.clone();
                    self.status_message =
                        format!("Update downloaded — restart to apply ({}).", info.latest);
                    self.update_status = UpdateStatus::Ready { info, new_exe: path };
                }
            }
            WorkerMsg::UpdateDownloadFailed(e) => {
                self.status_message = format!("Update download failed: {e}");
                // Common during a CI race: latest tag exists but the platform asset
                // hasn't been uploaded yet. Drop back to the "outdated, no asset"
                // modal so the user can open the release page, or wait and retry.
                if let UpdateStatus::Downloading { info, .. } = &self.update_status {
                    let info = info.clone();
                    self.update_status = UpdateStatus::Outdated(info);
                    self.update_modal_dismissed = false;
                } else {
                    self.update_status = UpdateStatus::Failed(e);
                }
            }
            WorkerMsg::FabricLoadersLoaded(v) => {
                self.fabric_loaders = v;
            }
            WorkerMsg::FabricLoadersFailed(e) => {
                tracing::warn!("fabric loader list failed: {e}");
            }
            WorkerMsg::FabricMcVersionsLoaded(v) => {
                self.fabric_mc_versions = v;
            }
            WorkerMsg::FabricMcVersionsFailed(e) => {
                tracing::warn!("fabric mc versions failed: {e}");
            }
            WorkerMsg::ModSearchStarted => {
                self.mod_search_loading = true;
                self.mod_search_error = None;
            }
            WorkerMsg::ModSearchDone {
                request_id,
                hits,
                total,
                offset,
                append,
            } => {
                // Stale response from an older search — discard so we don't
                // clobber what the user is actually looking at.
                if request_id != self.mod_search_request_id {
                    return;
                }
                self.mod_search_loading = false;
                self.mod_search_total = total;
                self.mod_search_api_offset = offset + hits.len() as u32;
                if append {
                    self.mod_search_results.extend(hits);
                } else {
                    self.mod_search_results = hits;
                    self.mod_search_offset = 0;
                }
            }
            WorkerMsg::ModSearchFailed { request_id, error } => {
                if request_id != self.mod_search_request_id {
                    return;
                }
                self.mod_search_loading = false;
                self.mod_search_error = Some(error.clone());
                self.status_message = format!("Mod search failed: {error}");
            }
            WorkerMsg::ModInstallStarted(p) => {
                self.mod_installing = Some(p);
            }
            WorkerMsg::ModInstallDone {
                project,
                filename,
                dep_count,
            } => {
                self.mod_installing = None;
                if !project.is_empty() {
                    let kind = self.browser_kind;
                    self.installed_meta.record(kind, project, filename.clone());
                    self.save_meta();
                }
                self.status_message = if dep_count == 0 {
                    format!("Installed: {filename}")
                } else {
                    format!("Installed: {filename}  (+{dep_count} dep{})", if dep_count == 1 { "" } else { "s" })
                };
                self.refresh_installed_mods();
            }
            WorkerMsg::ModInstallFailed { project: _, error } => {
                self.mod_installing = None;
                self.status_message = format!("Mod install failed: {error}");
            }
            WorkerMsg::CategoriesLoaded(c) => {
                self.categories = c;
            }
            WorkerMsg::CategoriesFailed(e) => {
                tracing::warn!("modrinth categories fetch failed: {e}");
            }
            WorkerMsg::VerifyDone { version, checked, repaired, missing } => {
                self.integrity_in_progress = false;
                // The verify worker shares the InstallProgress event stream with
                // installs; clear it so the Play tab stops showing "Verifying..."
                // and its progress bar after we're done.
                self.install = None;
                self.status_message = format!(
                    "Verify {version}: {checked} files checked, {repaired} re-fetched, {missing} still missing"
                );
            }
            WorkerMsg::VerifyFailed(e) => {
                self.integrity_in_progress = false;
                self.install = None;
                self.status_message = format!("Verify failed: {e}");
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

pub fn spawn_offline_skin_preview(
    client: reqwest::Client,
    tx: UnboundedSender<WorkerMsg>,
    input: String,
) {
    let input = input.trim().to_string();
    if input.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let url = match crate::skin::resolve_skin_url(&client, &input).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("offline skin resolve failed: {e}");
                return;
            }
        };
        match crate::skin::fetch_preview(&client, &url).await {
            Ok(p) => {
                let _ = tx.send(WorkerMsg::SkinPreviewLoaded(p));
            }
            Err(e) => tracing::warn!("offline skin preview fetch failed: {e}"),
        }
    });
}

pub fn spawn_skin_preview(
    client: reqwest::Client,
    tx: UnboundedSender<WorkerMsg>,
    uuid: String,
) {
    tokio::spawn(async move {
        let url = match crate::skin::current_skin_url(&client, &uuid).await {
            Ok(u) => u,
            Err(e) => {
                tracing::warn!("skin url lookup failed: {e}");
                return;
            }
        };
        match crate::skin::fetch_preview(&client, &url).await {
            Ok(p) => {
                let _ = tx.send(WorkerMsg::SkinPreviewLoaded(p));
            }
            Err(e) => tracing::warn!("skin preview fetch failed: {e}"),
        }
    });
}

pub fn spawn_news_fetch(client: reqwest::Client, tx: UnboundedSender<WorkerMsg>) {
    tokio::spawn(async move {
        match news::fetch(&client).await {
            Ok(entries) => {
                let _ = tx.send(WorkerMsg::NewsLoaded(entries));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::NewsFailed(format!("{e:#}")));
            }
        }
    });
}

pub fn spawn_article_fetch(
    client: reqwest::Client,
    tx: UnboundedSender<WorkerMsg>,
    index: usize,
    entry: NewsEntry,
) {
    tokio::spawn(async move {
        match news::fetch_article(&client, entry).await {
            Ok(article) => {
                let _ = tx.send(WorkerMsg::ArticleLoaded { index, article });
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::ArticleFailed {
                    index,
                    error: format!("{e:#}"),
                });
            }
        }
    });
}

pub fn spawn_modrinth_categories_fetch(client: reqwest::Client, tx: UnboundedSender<WorkerMsg>) {
    tokio::spawn(async move {
        match crate::modrinth::fetch_categories(&client).await {
            Ok(c) => {
                let _ = tx.send(WorkerMsg::CategoriesLoaded(c));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::CategoriesFailed(format!("{e:#}")));
            }
        }
    });
}

pub fn spawn_fabric_meta_fetch(client: reqwest::Client, tx: UnboundedSender<WorkerMsg>) {
    let c1 = client.clone();
    let tx1 = tx.clone();
    tokio::spawn(async move {
        match crate::fabric::fetch_loaders(&c1).await {
            Ok(entries) => {
                let versions: Vec<String> = entries
                    .into_iter()
                    .filter(|e| e.stable)
                    .map(|e| e.version)
                    .collect();
                let _ = tx1.send(WorkerMsg::FabricLoadersLoaded(versions));
            }
            Err(e) => {
                let _ = tx1.send(WorkerMsg::FabricLoadersFailed(format!("{e:#}")));
            }
        }
    });
    tokio::spawn(async move {
        match crate::fabric::fetch_supported_mc_versions(&client).await {
            Ok(v) => {
                let _ = tx.send(WorkerMsg::FabricMcVersionsLoaded(v));
            }
            Err(e) => {
                let _ = tx.send(WorkerMsg::FabricMcVersionsFailed(format!("{e:#}")));
            }
        }
    });
}

