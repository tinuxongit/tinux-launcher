mod app;
mod auth;
mod config;
mod download;
mod event;
mod java;
mod launch;
mod logging;
mod manifest;
mod news;
mod paths;
mod skin;
mod theme;
mod ui;
mod version;
mod worker;

use anyhow::Result;
use app::{AccountMode, App, Focus, LaunchState, SkinModel};
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use event::{Hit, Tab};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};
use tokio::sync::mpsc::unbounded_channel;

#[tokio::main]
async fn main() -> Result<()> {
    let paths = paths::Paths::resolve()?;
    let _log_guard = logging::init(&paths.logs)?;
    config::ensure_stub();

    tracing::info!("Tinux Launcher starting; data dir: {}", paths.root.display());

    let (worker_tx, mut worker_rx) = unbounded_channel();
    let mut app = App::new(paths, worker_tx.clone());
    app::spawn_manifest_fetch(app.client.clone(), worker_tx.clone());
    app::spawn_news_fetch(app.client.clone(), worker_tx.clone());

    let mut terminal = setup_terminal()?;
    terminal.clear()?;
    let result = run_loop(&mut terminal, &mut app, &mut worker_rx).await;
    restore_terminal(&mut terminal)?;

    if let Err(e) = &result {
        eprintln!("error: {e:#}");
    }
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    worker_rx: &mut tokio::sync::mpsc::UnboundedReceiver<event::WorkerMsg>,
) -> Result<()> {
    let mut input = EventStream::new();
    let mut last_tab = app.tab;

    while app.running {
        if app.tab != last_tab {
            app.needs_clear = true;
            last_tab = app.tab;
        }
        if app.needs_clear {
            terminal.clear()?;
            app.needs_clear = false;
        }
        terminal.draw(|f| ui::draw(f, app))?;

        tokio::select! {
            ev = input.next() => {
                match ev {
                    Some(Ok(Event::Key(k))) => handle_key(app, k),
                    Some(Ok(Event::Mouse(m))) => handle_mouse(app, m),
                    Some(Ok(Event::Resize(_, _))) => {}
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::warn!("event stream error: {e}");
                    }
                    None => break,
                }
            }
            Some(msg) = worker_rx.recv() => {
                app.handle_worker(msg);
            }
        }
    }
    Ok(())
}

fn handle_key(app: &mut App, k: KeyEvent) {
    if k.kind != KeyEventKind::Press {
        return;
    }

    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('l') => {
                app.needs_clear = true;
                return;
            }
            KeyCode::Char('c') => {
                copy_selected_log(app);
                return;
            }
            KeyCode::Char('a') => {
                if app.tab == Tab::Logs && !app.logs.is_empty() {
                    app.selected_log_range = Some((0, app.logs.len() - 1));
                    app.status_message = format!("Selected all {} log lines", app.logs.len());
                }
                return;
            }
            KeyCode::Char('v') => {
                paste_offline_name(app);
                return;
            }
            _ => {}
        }
    }

    if app.focus == Focus::OfflineName {
        let mut changed = false;
        match k.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.focus = Focus::None;
            }
            KeyCode::Backspace => {
                if app.offline_name.pop().is_some() {
                    changed = true;
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                if app.offline_name.chars().count() < 16 {
                    app.offline_name.push(c);
                    changed = true;
                }
            }
            _ => {}
        }
        if changed {
            config::save_offline_name(&app.offline_name);
        }
        return;
    }

    if app.focus == Focus::SkinUrl {
        match k.code {
            KeyCode::Esc | KeyCode::Enter => app.focus = Focus::None,
            KeyCode::Backspace => {
                app.skin_url_input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if app.skin_url_input.chars().count() < 250 {
                    app.skin_url_input.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    match k.code {
        KeyCode::Esc if app.viewing_news.is_some() => {
            app.viewing_news = None;
            app.article = None;
            app.article_loading = false;
        }
        KeyCode::Esc | KeyCode::Char('q') => app.running = false,
        KeyCode::Char('1') => app.tab = Tab::Play,
        KeyCode::Char('2') => app.tab = Tab::Versions,
        KeyCode::Char('3') => app.tab = Tab::Profile,
        KeyCode::Char('4') => app.tab = Tab::Logs,
        KeyCode::Tab => {
            app.tab = match app.tab {
                Tab::Play => Tab::Versions,
                Tab::Versions => Tab::Profile,
                Tab::Profile => Tab::Logs,
                Tab::Logs => Tab::Play,
            };
        }
        KeyCode::Up => match app.tab {
            Tab::Versions => app.list_offset = app.list_offset.saturating_sub(1),
            Tab::Logs => app.log_offset = app.log_offset.saturating_add(1),
            _ => {}
        },
        KeyCode::Down => match app.tab {
            Tab::Versions => app.list_offset = app.list_offset.saturating_add(1),
            Tab::Logs => app.log_offset = app.log_offset.saturating_sub(1),
            _ => {}
        },
        KeyCode::PageUp => match app.tab {
            Tab::Versions => app.list_offset = app.list_offset.saturating_sub(10),
            Tab::Logs => app.log_offset = app.log_offset.saturating_add(10),
            _ => {}
        },
        KeyCode::PageDown => match app.tab {
            Tab::Versions => app.list_offset = app.list_offset.saturating_add(10),
            Tab::Logs => app.log_offset = app.log_offset.saturating_sub(10),
            _ => {}
        },
        KeyCode::Enter if app.tab == Tab::Play => trigger_launch(app),
        _ => {}
    }
}

fn handle_mouse(app: &mut App, m: MouseEvent) {
    match m.kind {
        MouseEventKind::Moved => {
            app.hover = ui::hit_test(app, m.column, m.row);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some(hit) = ui::hit_test(app, m.column, m.row) {
                if hit == Hit::NewsSplitter {
                    app.dragging_split = true;
                    return;
                }
                let extend = m.modifiers.contains(KeyModifiers::CONTROL);
                dispatch(app, hit, extend);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            app.dragging_split = false;
        }
        MouseEventKind::Drag(MouseButton::Left) if app.dragging_split => {
            let inner = app.play_inner;
            if inner.height > 8 && m.row > inner.y {
                let rel = m.row - inner.y;
                let min = 7u16;
                let max = inner.height.saturating_sub(4);
                app.news_split_top = Some(rel.clamp(min, max));
            }
        }
        MouseEventKind::ScrollUp => {
            if app.viewing_news.is_some() {
                app.article_offset = app.article_offset.saturating_sub(2);
            } else {
                match app.tab {
                    Tab::Versions => app.list_offset = app.list_offset.saturating_sub(3),
                    Tab::Logs => app.log_offset = app.log_offset.saturating_add(3),
                    Tab::Play => app.news_offset = app.news_offset.saturating_sub(2),
                    _ => {}
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if app.viewing_news.is_some() {
                app.article_offset = app.article_offset.saturating_add(2);
            } else {
                match app.tab {
                    Tab::Versions => app.list_offset = app.list_offset.saturating_add(3),
                    Tab::Logs => app.log_offset = app.log_offset.saturating_sub(3),
                    Tab::Play => app.news_offset = app.news_offset.saturating_add(2),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn dispatch(app: &mut App, hit: Hit, extend: bool) {
    let prev_focus = app.focus;
    if hit != Hit::OfflineNameField && hit != Hit::SkinUrlField {
        app.focus = Focus::None;
    }
    if prev_focus == Focus::OfflineName && app.focus != Focus::OfflineName {
        config::save_offline_name(&app.offline_name);
    }
    match hit {
        Hit::Tab(t) => app.tab = t,
        Hit::FilterReleases => {
            app.filter = app::VersionFilter::Releases;
            app.list_offset = 0;
        }
        Hit::FilterSnapshots => {
            app.filter = app::VersionFilter::Snapshots;
            app.list_offset = 0;
        }
        Hit::FilterOld => {
            app.filter = app::VersionFilter::Old;
            app.list_offset = 0;
        }
        Hit::VersionRow(i) => {
            let visible = app.visible_versions();
            if let Some(v) = visible.get(i) {
                app.selected_version = Some(v.id.clone());
            }
        }
        Hit::LaunchButton => trigger_launch(app),
        Hit::InstallButton => trigger_install(app),
        Hit::LoginButton => trigger_login(app),
        Hit::LogoutButton => {
            auth::logout();
            app.account = None;
            app.status_message = "Signed out".into();
        }
        Hit::OfflineNameField => {
            app.focus = Focus::OfflineName;
        }
        Hit::LogRow(i) => {
            if extend {
                if let Some((anchor, _)) = app.selected_log_range {
                    app.selected_log_range = Some((anchor, i));
                } else {
                    app.selected_log_range = Some((i, i));
                }
            } else {
                app.selected_log_range = Some((i, i));
            }
        }
        Hit::CopyLineButton => copy_selected_log(app),
        Hit::CopyAllButton => copy_all_logs(app),
        Hit::ModeOffline => app.account_mode = AccountMode::Offline,
        Hit::ModeOnline => app.account_mode = AccountMode::Online,
        Hit::NewsItem(i) => {
            if let Some(entry) = app.news.get(i).cloned() {
                let title = entry.title.clone();
                app.viewing_news = Some(i);
                app.article = None;
                app.article_loading = true;
                app.article_offset = 0;
                app.status_message = format!("Loading: {title}");
                let client = app.client.clone();
                let tx = app.worker_tx.clone();
                app::spawn_article_fetch(client, tx, i, entry);
            }
        }
        Hit::CloseArticle => {
            app.viewing_news = None;
            app.article = None;
            app.article_loading = false;
        }
        Hit::OpenArticleExternal => {
            if let Some(art) = &app.article {
                if !art.read_more_link.is_empty() {
                    let _ = webbrowser::open(&art.read_more_link);
                    app.status_message = "Opened article in browser".into();
                }
            }
        }
        Hit::OpenAllArticles => {
            let _ = webbrowser::open("https://www.minecraft.net/en-us/articles");
            app.status_message = "Opened minecraft.net/articles".into();
        }
        Hit::NewsSplitter => {} // drag handled in handle_mouse
        Hit::SkinUrlField => app.focus = Focus::SkinUrl,
        Hit::SkinModelClassic => app.skin_model = SkinModel::Classic,
        Hit::SkinModelSlim => app.skin_model = SkinModel::Slim,
        Hit::ApplySkinButton => trigger_apply_skin(app),
        Hit::ResetSkinButton => trigger_reset_skin(app),
        Hit::PreviewSkinButton => trigger_preview_skin(app),
        Hit::ClearPreviewButton => {
            app.skin_pending_preview = None;
            app.status_message = "Preview cleared".into();
        }
    }
}

fn trigger_login(app: &mut App) {
    if app.auth_in_progress {
        return;
    }
    let tx = app.worker_tx.clone();
    let _ = tx.send(event::WorkerMsg::AuthStarted);
    tokio::spawn(async move {
        match auth::interactive_login().await {
            Ok(a) => {
                let _ = tx.send(event::WorkerMsg::AuthSucceeded(a));
            }
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::AuthFailed(format!("{e:#}")));
            }
        }
    });
}

fn trigger_install(app: &mut App) {
    if app.install_in_progress() {
        app.status_message = "Install already running".into();
        return;
    }
    let Some(entry) = app.selected_manifest_entry() else {
        app.status_message = "Pick a version first (Versions tab)".into();
        return;
    };
    let client = app.client.clone();
    let paths_clone = clone_paths(&app.paths);
    let tx = app.worker_tx.clone();
    tokio::spawn(async move {
        worker::do_install(client, paths_clone, entry, tx).await;
    });
}

fn trigger_launch(app: &mut App) {
    if app.launch_state == LaunchState::Running {
        return;
    }
    if app.install_in_progress() {
        app.status_message = "Install in progress — wait for it to finish".into();
        return;
    }
    let Some(entry) = app.selected_manifest_entry() else {
        app.status_message = "Pick a version first (Versions tab)".into();
        return;
    };
    let Some(java) = app.java.clone() else {
        app.status_message = "Java not detected — install Java 17+ and restart".into();
        return;
    };
    let opts = match app.account_mode {
        AccountMode::Online => match &app.account {
            Some(a) => launch::LaunchOptions::from_account(a),
            None => {
                app.status_message = "Sign in first, or switch to Offline mode".into();
                return;
            }
        },
        AccountMode::Offline => launch::LaunchOptions::offline(app.offline_name.clone()),
    };

    let client = app.client.clone();
    let paths_clone = clone_paths(&app.paths);
    let tx = app.worker_tx.clone();
    tokio::spawn(async move {
        worker::do_install_and_launch(client, paths_clone, entry, java, opts, tx).await;
    });
}

fn copy_to_clipboard(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut cb| cb.set_text(text))
        .map_err(|e| e.to_string())
}

fn copy_selected_log(app: &mut App) {
    let Some((a, b)) = app.selected_log_range else {
        app.status_message = "Click a log line first (Ctrl+click extends).".into();
        return;
    };
    let (lo, hi) = (a.min(b), a.max(b));
    let lines: Vec<String> = app
        .logs
        .iter()
        .enumerate()
        .filter(|(i, _)| *i >= lo && *i <= hi)
        .map(|(_, l)| l.clone())
        .collect();
    if lines.is_empty() {
        app.status_message = "Selected lines no longer exist".into();
        app.selected_log_range = None;
        return;
    }
    let n = lines.len();
    let text = lines.join("\n");
    match copy_to_clipboard(text) {
        Ok(()) => {
            let s = if n == 1 { "" } else { "s" };
            app.status_message = format!("Copied {n} line{s} to clipboard");
        }
        Err(e) => app.status_message = format!("Clipboard error: {e}"),
    }
}

fn trigger_preview_skin(app: &mut App) {
    let input = app.skin_url_input.trim().to_string();
    if input.is_empty() {
        app.status_message = "Paste a skin URL or a Minecraft username first".into();
        return;
    }
    if app.skin_pending_loading {
        return;
    }
    app.skin_pending_loading = true;
    app.skin_error = None;
    app.status_message = "Fetching preview...".into();
    let client = app.client.clone();
    let tx = app.worker_tx.clone();
    tokio::spawn(async move {
        let url = match skin::resolve_skin_url(&client, &input).await {
            Ok(u) => u,
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::SkinPendingFailed(format!("{e:#}")));
                return;
            }
        };
        match skin::fetch_preview(&client, &url).await {
            Ok(p) => {
                let _ = tx.send(event::WorkerMsg::SkinPendingLoaded(p));
            }
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::SkinPendingFailed(format!("{e:#}")));
            }
        }
    });
}

fn trigger_apply_skin(app: &mut App) {
    let input = app.skin_url_input.trim().to_string();
    if input.is_empty() {
        app.status_message = "Paste a skin URL or a Minecraft username first".into();
        return;
    }
    if app.skin_busy {
        return;
    }
    app.skin_busy = true;
    app.skin_error = None;
    app.status_message = "Resolving skin...".into();

    let client = app.client.clone();
    let tx = app.worker_tx.clone();
    let offline = app.account_mode == AccountMode::Offline || app.account.is_none();
    let token = app.account.as_ref().map(|a| a.access_token.clone());
    let uuid = app.account.as_ref().map(|a| a.uuid.clone());
    let model = app.skin_model.as_str().to_string();

    tokio::spawn(async move {
        let url = match skin::resolve_skin_url(&client, &input).await {
            Ok(u) => u,
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::SkinFailed(format!("{e:#}")));
                return;
            }
        };

        if offline {
            crate::config::save_offline_skin_url(&url);
            let _ = tx.send(event::WorkerMsg::SkinApplied);
            return;
        }

        let (token, uuid) = match (token, uuid) {
            (Some(t), Some(u)) => (t, u),
            _ => {
                let _ = tx.send(event::WorkerMsg::SkinFailed("Sign in first".into()));
                return;
            }
        };
        match auth::set_skin_url(&client, &token, &model, &url).await {
            Ok(()) => {
                let _ = tx.send(event::WorkerMsg::SkinApplied);
                app::spawn_skin_preview(client, tx, uuid);
            }
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::SkinFailed(format!("{e:#}")));
            }
        }
    });
}

fn trigger_reset_skin(app: &mut App) {
    let Some(acct) = app.account.clone() else {
        app.status_message = "Sign in first".into();
        return;
    };
    if app.skin_busy {
        return;
    }
    app.skin_busy = true;
    app.status_message = "Resetting skin...".into();
    let client = app.client.clone();
    let token = acct.access_token;
    let uuid = acct.uuid;
    let tx = app.worker_tx.clone();
    tokio::spawn(async move {
        match auth::reset_skin(&client, &token).await {
            Ok(()) => {
                let _ = tx.send(event::WorkerMsg::SkinApplied);
                app::spawn_skin_preview(client, tx, uuid);
            }
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::SkinFailed(format!("{e:#}")));
            }
        }
    });
}

fn paste_offline_name(app: &mut App) {
    if app.focus != Focus::OfflineName {
        return;
    }
    let Ok(mut cb) = arboard::Clipboard::new() else {
        return;
    };
    let Ok(text) = cb.get_text() else {
        return;
    };
    for c in text.chars() {
        if c.is_ascii() && !c.is_control() && app.offline_name.chars().count() < 16 {
            app.offline_name.push(c);
        }
    }
}

fn copy_all_logs(app: &mut App) {
    let all: String = app
        .logs
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let n = app.logs.len();
    match copy_to_clipboard(all) {
        Ok(()) => app.status_message = format!("Copied {n} log lines to clipboard"),
        Err(e) => app.status_message = format!("Clipboard error: {e}"),
    }
}

fn clone_paths(p: &paths::Paths) -> paths::Paths {
    paths::Paths {
        root: p.root.clone(),
        versions: p.versions.clone(),
        libraries: p.libraries.clone(),
        assets: p.assets.clone(),
        assets_indexes: p.assets_indexes.clone(),
        assets_objects: p.assets_objects.clone(),
        natives: p.natives.clone(),
        instances: p.instances.clone(),
        logs: p.logs.clone(),
        cache: p.cache.clone(),
    }
}
