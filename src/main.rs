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
mod theme;
mod ui;
mod version;
mod worker;

use anyhow::Result;
use app::{AccountMode, App, Focus, LaunchState};
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
        match k.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.focus = Focus::None;
            }
            KeyCode::Backspace => {
                app.offline_name.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if app.offline_name.chars().count() < 16 {
                    app.offline_name.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    match k.code {
        KeyCode::Esc | KeyCode::Char('q') => app.running = false,
        KeyCode::Char('1') => app.tab = Tab::Play,
        KeyCode::Char('2') => app.tab = Tab::Versions,
        KeyCode::Char('3') => app.tab = Tab::Accounts,
        KeyCode::Char('4') => app.tab = Tab::Logs,
        KeyCode::Tab => {
            app.tab = match app.tab {
                Tab::Play => Tab::Versions,
                Tab::Versions => Tab::Accounts,
                Tab::Accounts => Tab::Logs,
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
                let extend = m.modifiers.contains(KeyModifiers::CONTROL);
                dispatch(app, hit, extend);
            }
        }
        MouseEventKind::ScrollUp => match app.tab {
            Tab::Versions => app.list_offset = app.list_offset.saturating_sub(3),
            Tab::Logs => app.log_offset = app.log_offset.saturating_add(3),
            _ => {}
        },
        MouseEventKind::ScrollDown => match app.tab {
            Tab::Versions => app.list_offset = app.list_offset.saturating_add(3),
            Tab::Logs => app.log_offset = app.log_offset.saturating_sub(3),
            _ => {}
        },
        _ => {}
    }
}

fn dispatch(app: &mut App, hit: Hit, extend: bool) {
    if hit != Hit::OfflineNameField {
        app.focus = Focus::None;
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
        Hit::OpenConfigButton => open_config(app),
        Hit::ModeOffline => app.account_mode = AccountMode::Offline,
        Hit::ModeOnline => app.account_mode = AccountMode::Online,
        Hit::NewsItem(i) => {
            if let Some(entry) = app.news.get(i) {
                if !entry.read_more_link.is_empty() {
                    let _ = webbrowser::open(&entry.read_more_link);
                    app.status_message = format!("Opened news: {}", entry.title);
                }
            }
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
    let Some(entry) = app.selected_manifest_entry() else {
        app.status_message = "Pick a version first (Versions tab)".into();
        return;
    };
    if app.install.is_some() {
        return;
    }
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

fn open_config(app: &mut App) {
    let Some(p) = config::path() else {
        app.status_message = "Could not resolve config path".into();
        return;
    };
    match config::open_with_default_app(&p) {
        Ok(()) => app.status_message = format!("Opened {}", p.display()),
        Err(e) => app.status_message = format!("Could not open config: {e}"),
    }
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
