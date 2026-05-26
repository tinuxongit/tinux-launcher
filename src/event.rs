use crate::auth::Account;
use crate::manifest::VersionManifest;
use crate::news::{Article, NewsEntry};
use crate::skin::SkinPreview;
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
    FilterSnapshots,
    FilterOld,
    OfflineNameField,
    LogRow(usize),
    CopyLineButton,
    CopyAllButton,
    ModeOffline,
    ModeOnline,
    NewsItem(usize),
    CloseArticle,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Play,
    Versions,
    Profile,
    Logs,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Play, Tab::Versions, Tab::Profile, Tab::Logs];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Play => "Play",
            Tab::Versions => "Versions",
            Tab::Profile => "Profile",
            Tab::Logs => "Logs",
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
}
