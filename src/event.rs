use crate::auth::Account;
use crate::manifest::VersionManifest;
use crate::modrinth::{Category, SearchHit};
use crate::news::{Article, NewsEntry};
use crate::skin::SkinPreview;
use crate::update::UpdateInfo;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Install,
    Verify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hit {
    Tab(Tab),
    LoginButton,
    LogoutButton,
    LaunchButton,
    InstallButton,
    VersionRow(usize),
    FilterReleases,
    FilterModded,
    ToggleShowSnapshots,
    ToggleShowOlder,
    BrowseModsButton,
    ModResult(usize),
    RemoveModButton(usize),
    BrowserTabMods,
    BrowserTabShaders,
    BrowserTabResourcePacks,
    ShowMoreModsButton,
    DismissInfoPopup,
    CategoryChip(usize),
    OpenFiltersButton,
    CloseFiltersPopup,
    ClearAllFilters,
    CloseModBrowser,
    ModSearchField,
    OfflineNameField,
    LogRow(usize),
    CopyLineButton,
    CopyAllButton,
    ModeOffline,
    ModeOnline,
    NewsItem(usize),
    NewsScrollbar,
    CloseArticle,
    ArticleScrollbar,
    OpenArticleExternal,
    OpenAllArticles,
    NewsSplitter,
    SkinUrlField,
    SkinModelClassic,
    SkinModelSlim,
    ApplySkinButton,
    ResetSkinButton,
    PreviewSkinButton,
    ClearPreviewButton,
    RotateSkinLeft,
    RotateSkinRight,
    VersionsScrollbar,
    LogsScrollbar,
    CheckUpdatesButton,
    OpenReleasesPage,
    InstallUpdateNow,
    DismissUpdate,
    RamDecrease,
    RamIncrease,
    JavaPathField,
    ClearJavaPath,
    JavaPathForVersionField,
    ClearJavaPathForVersion,
    OpenDataFolder,
    VerifyIntegrityButton,
    InstalledFilterToggle,
    ExportProfileButton,
    ImportProfileButton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Play,
    Versions,
    Profile,
    Logs,
    Settings,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Play,
        Tab::Versions,
        Tab::Profile,
        Tab::Logs,
        Tab::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Play => "Play",
            Tab::Versions => "Versions",
            Tab::Profile => "Profile",
            Tab::Logs => "Logs",
            Tab::Settings => "Settings",
        }
    }
}

#[derive(Debug)]
pub enum WorkerMsg {
    ManifestLoaded(Arc<VersionManifest>),
    ManifestFailed(String),
    AuthStarted,
    AuthSucceeded(Account),
    AuthFailed(String),
    InstallProgress {
        kind: InstallKind,
        done: u64,
        total: u64,
        what: String,
    },
    InstallDone(String),
    InstallFailed { version: String, error: String },
    LaunchStarted(String),
    LaunchLog(String),
    LaunchExited(i32),
    LaunchFailed(String),
    NewsLoaded(Vec<NewsEntry>),
    NewsFailed(String),
    ArticleLoaded { index: usize, article: Article },
    ArticleFailed { index: usize, error: String },
    SkinPreviewLoaded(SkinPreview),
    SkinPendingLoaded(SkinPreview),
    SkinPendingFailed(String),
    SkinApplied,
    SkinFailed(String),
    UpdateCheckStarted,
    UpdateCheckResult(UpdateInfo),
    UpdateCheckFailed(String),
    UpdateDownloadStarted,
    UpdateDownloadProgress { done: u64, total: u64 },
    UpdateDownloaded(std::path::PathBuf),
    UpdateDownloadFailed(String),
    FabricLoadersLoaded(Vec<String>),
    FabricLoadersFailed(String),
    FabricMcVersionsLoaded(Vec<String>),
    FabricMcVersionsFailed(String),
    ModSearchStarted,
    ModSearchDone {
        request_id: u64,
        hits: Vec<SearchHit>,
        total: u32,
        offset: u32,
        append: bool,
    },
    ModSearchFailed {
        request_id: u64,
        error: String,
    },
    ModInstallStarted(String),
    ModInstallDone {
        #[allow(dead_code)]
        project: String,
        filename: String,
        dep_count: usize,
    },
    ModInstallFailed {
        #[allow(dead_code)]
        project: String,
        error: String,
    },
    CategoriesLoaded(Vec<Category>),
    CategoriesFailed(String),
    VerifyDone {
        version: String,
        checked: usize,
        repaired: usize,
        missing: usize,
    },
    VerifyFailed(String),
}
