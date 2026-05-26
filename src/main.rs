mod app;
mod auth;
mod config;
mod download;
mod event;
mod fabric;
mod java;
mod launch;
mod logging;
mod manifest;
mod meta;
mod modrinth;
mod news;
mod paths;
mod skin;
mod theme;
mod ui;
mod update;
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
use ratatui::{backend::CrosstermBackend, layout::Rect, Terminal};
use std::io::{self, Stdout};
use tokio::sync::mpsc::unbounded_channel;

const TERMINAL_COLS: u16 = 120;
const TERMINAL_ROWS: u16 = 38;

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
    app::spawn_fabric_meta_fetch(app.client.clone(), worker_tx.clone());
    app::spawn_modrinth_categories_fetch(app.client.clone(), worker_tx.clone());
    update::spawn_check(app.client.clone(), worker_tx.clone());
    // If the user has previously set an offline skin URL/username, render its
    // preview right away so the Profile tab isn't empty.
    app::spawn_offline_skin_preview(
        app.client.clone(),
        worker_tx.clone(),
        app.skin_url_input.clone(),
    );

    let (mut terminal, original_size) = setup_terminal()?;
    terminal.clear()?;
    let result = run_loop(&mut terminal, &mut app, &mut worker_rx).await;
    restore_terminal(&mut terminal, original_size)?;

    if let Err(e) = &result {
        eprintln!("error: {e:#}");
    }
    result
}

fn setup_terminal() -> Result<(Terminal<CrosstermBackend<Stdout>>, Option<(u16, u16)>)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    // Keep the TUI at a predictable size while it is running.
    // Some terminal hosts ignore resize requests, so this is best-effort.
    let original_size = crossterm::terminal::size().ok();
    enforce_terminal_size(&mut stdout);
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok((terminal, original_size))
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    original_size: Option<(u16, u16)>,
) -> Result<()> {
    disable_raw_mode()?;
    if let Some((cols, rows)) = original_size {
        let _ = execute!(terminal.backend_mut(), crossterm::terminal::SetSize(cols, rows));
    }
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn enforce_terminal_size(stdout: &mut Stdout) {
    if crossterm::terminal::size().ok() != Some((TERMINAL_COLS, TERMINAL_ROWS)) {
        let _ = execute!(
            stdout,
            crossterm::terminal::SetSize(TERMINAL_COLS, TERMINAL_ROWS)
        );
    }
}

fn enforce_terminal_backend_size(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    if crossterm::terminal::size().ok() != Some((TERMINAL_COLS, TERMINAL_ROWS)) {
        let _ = execute!(
            terminal.backend_mut(),
            crossterm::terminal::SetSize(TERMINAL_COLS, TERMINAL_ROWS)
        );
        let _ = terminal.clear();
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    worker_rx: &mut tokio::sync::mpsc::UnboundedReceiver<event::WorkerMsg>,
) -> Result<()> {
    let mut input = EventStream::new();
    let mut last_tab = app.tab;
    // The SetSize on startup can land after the first draw, leaving stale
    // cells from the host terminal's previous content. Force a clear for
    // the first several frames to paint those over.
    let mut startup_clears: u8 = 4;

    while app.running {
        if app.tab != last_tab {
            app.needs_clear = true;
            last_tab = app.tab;
        }
        if startup_clears > 0 {
            app.needs_clear = true;
            startup_clears -= 1;
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
                    Some(Ok(Event::Resize(_, _))) => enforce_terminal_backend_size(terminal),
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

    if app.focus == Focus::ModSearch {
        match k.code {
            KeyCode::Esc => app.focus = Focus::None,
            KeyCode::Enter => {
                app.focus = Focus::None;
                trigger_mod_search(app, false);
            }
            KeyCode::Backspace => {
                app.mod_search_query.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if app.mod_search_query.chars().count() < 80 {
                    app.mod_search_query.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    if app.focus == Focus::JavaPath {
        match k.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.focus = Focus::None;
                config::save_java_path(&app.java_path_input);
            }
            KeyCode::Backspace => {
                app.java_path_input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if app.java_path_input.chars().count() < 260 {
                    app.java_path_input.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    if app.focus == Focus::JavaPathForVersion {
        match k.code {
            KeyCode::Esc | KeyCode::Enter => {
                app.focus = Focus::None;
                if let Some(id) = app.java_path_for_version_id.clone() {
                    config::save_java_path_for(&id, &app.java_path_for_version_input);
                    if app.java_path_for_version_input.trim().is_empty() {
                        app.java_path_per_version.remove(&id);
                    } else {
                        app.java_path_per_version
                            .insert(id, app.java_path_for_version_input.clone());
                    }
                }
            }
            KeyCode::Backspace => {
                app.java_path_for_version_input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if app.java_path_for_version_input.chars().count() < 260 {
                    app.java_path_for_version_input.push(c);
                }
            }
            _ => {}
        }
        return;
    }

    match k.code {
        // Esc dismisses the topmost modal first (matches visual z-order in ui::draw):
        // info_popup > update_modal > filters_popup > mod_browser > article > tab default.
        KeyCode::Esc if app.info_popup.is_some() => {
            app.info_popup = None;
        }
        KeyCode::Esc if update_modal_visible(app) => {
            app.update_modal_dismissed = true;
        }
        KeyCode::Esc if app.filters_popup_open => {
            app.filters_popup_open = false;
        }
        KeyCode::Esc if app.mod_browser_open => {
            app.mod_browser_open = false;
        }
        KeyCode::Esc if app.viewing_news.is_some() => {
            app.viewing_news = None;
            app.article = None;
            app.article_loading = false;
        }
        KeyCode::Esc | KeyCode::Char('q') => app.running = false,
        KeyCode::Char('1') => { app.tab = Tab::Play; app.focus = Focus::None; }
        KeyCode::Char('2') => { app.tab = Tab::Versions; app.focus = Focus::None; }
        KeyCode::Char('3') => { app.tab = Tab::Profile; app.focus = Focus::None; }
        KeyCode::Char('4') => { app.tab = Tab::Logs; app.focus = Focus::None; }
        KeyCode::Char('5') => { app.tab = Tab::Settings; app.focus = Focus::None; }
        KeyCode::Tab => {
            app.tab = match app.tab {
                Tab::Play => Tab::Versions,
                Tab::Versions => Tab::Profile,
                Tab::Profile => Tab::Logs,
                Tab::Logs => Tab::Settings,
                Tab::Settings => Tab::Play,
            };
            app.focus = Focus::None;
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
            if let Some((rect, hit)) = ui::hit_region(app, m.column, m.row) {
                if hit == Hit::NewsSplitter {
                    app.dragging_split = true;
                    return;
                }
                if is_scrollbar_hit(hit) {
                    app.dragging_scrollbar = Some((hit, rect));
                    scroll_to_mouse(app, hit, rect, m.row);
                    return;
                }
                let extend = m.modifiers.contains(KeyModifiers::CONTROL);
                dispatch(app, hit, extend);
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            app.dragging_split = false;
            app.dragging_scrollbar = None;
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
        MouseEventKind::Drag(MouseButton::Left) => {
            if let Some((hit, rect)) = app.dragging_scrollbar {
                scroll_to_mouse(app, hit, rect, m.row);
            }
        }
        MouseEventKind::ScrollUp => {
            if app.filters_popup_open {
                app.filters_scroll = app.filters_scroll.saturating_sub(2);
            } else if app.mod_browser_open {
                app.mod_search_offset = app.mod_search_offset.saturating_sub(2);
            } else if app.viewing_news.is_some() {
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
            if app.filters_popup_open {
                app.filters_scroll = app.filters_scroll.saturating_add(2);
            } else if app.mod_browser_open {
                app.mod_search_offset = app.mod_search_offset.saturating_add(2);
            } else if app.viewing_news.is_some() {
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

fn is_scrollbar_hit(hit: Hit) -> bool {
    matches!(
        hit,
        Hit::NewsScrollbar | Hit::ArticleScrollbar | Hit::VersionsScrollbar | Hit::LogsScrollbar
    )
}

fn scroll_to_mouse(app: &mut App, hit: Hit, rect: Rect, row: u16) {
    let Some(pos) = scrollbar_position(rect, row) else {
        return;
    };
    match hit {
        Hit::NewsScrollbar => {
            let total = app.news.len();
            let visible = rect.height as usize;
            if total > visible {
                app.news_offset = pos_for_range(pos, total - visible);
            }
        }
        Hit::VersionsScrollbar => {
            let total = app.visible_versions().len();
            let visible = rect.height as usize;
            if total > visible {
                app.list_offset = pos_for_range(pos, total - visible);
            }
        }
        Hit::LogsScrollbar => {
            let total = app.logs.len();
            let visible = rect.height as usize;
            if total > visible {
                let max = total - visible;
                app.log_offset = max.saturating_sub(pos_for_range(pos, max));
            }
        }
        Hit::ArticleScrollbar => {
            if let Some(article) = &app.article {
                let total = ui::article_line_count(article);
                let visible = rect.height as usize;
                if total > visible {
                    app.article_offset = pos_for_range(pos, total - visible) as u16;
                }
            }
        }
        _ => {}
    }
}

fn scrollbar_position(rect: Rect, row: u16) -> Option<(usize, usize)> {
    if rect.height == 0 {
        return None;
    }
    let last = rect.height.saturating_sub(1);
    let rel = row.saturating_sub(rect.y).min(last) as usize;
    Some((rel, last.max(1) as usize))
}

fn pos_for_range((rel, denom): (usize, usize), max: usize) -> usize {
    if max == 0 {
        0
    } else {
        (rel * max + denom / 2) / denom
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
        Hit::Tab(t) => {
            app.tab = t;
            // Drop input focus so keystrokes on the new tab don't route to
            // hidden fields (e.g. the mod search field on a Versions tab).
            app.focus = Focus::None;
        }
        Hit::FilterReleases => {
            app.filter = app::VersionFilter::Releases;
            app.list_offset = 0;
        }
        Hit::FilterModded => {
            app.filter = app::VersionFilter::Modded;
            app.list_offset = 0;
        }
        Hit::ToggleShowSnapshots => {
            app.show_snapshots = !app.show_snapshots;
            app.list_offset = 0;
            config::save_version_toggles(app.show_snapshots, app.show_older);
        }
        Hit::ToggleShowOlder => {
            app.show_older = !app.show_older;
            app.list_offset = 0;
            config::save_version_toggles(app.show_snapshots, app.show_older);
        }
        Hit::BrowseModsButton => {
            if app.selected_modded_installed() {
                app.mod_browser_open = true;
                app.browser_kind = app::ContentKind::Mods;
                app.reload_meta();
                app.refresh_installed_mods();
                trigger_mod_search(app, false);
            } else {
                app.info_popup = Some(app::InfoPopup::new(
                    "Install this version first",
                    "Click the Install button on the Play tab to download and set up Fabric, then come back here to browse mods.",
                ));
            }
        }
        Hit::CloseModBrowser => {
            app.mod_browser_open = false;
        }
        Hit::ModSearchField => {
            app.focus = Focus::ModSearch;
        }
        Hit::ModResult(i) => {
            trigger_mod_install(app, i);
        }
        Hit::RemoveModButton(i) => {
            trigger_mod_remove(app, i);
        }
        Hit::BrowserTabMods => switch_browser_tab(app, app::ContentKind::Mods),
        Hit::BrowserTabShaders => {
            if app.shaders_available() {
                switch_browser_tab(app, app::ContentKind::Shaders);
            } else {
                app.info_popup = Some(app::InfoPopup::new(
                    "Shader loader needed",
                    "Install Iris or Oculus from the Mods tab. Once a shader-loader jar is present (or you've launched the game once with one installed), this tab unlocks.",
                ));
            }
        }
        Hit::BrowserTabResourcePacks => switch_browser_tab(app, app::ContentKind::ResourcePacks),
        Hit::ShowMoreModsButton => {
            trigger_mod_search(app, true);
        }
        Hit::DismissInfoPopup => {
            app.info_popup = None;
        }
        Hit::InstalledFilterToggle => {
            app.installed_filter_only = !app.installed_filter_only;
            app.status_message = if app.installed_filter_only {
                "Showing only installed projects".into()
            } else {
                "Showing all results".into()
            };
        }
        Hit::ExportProfileButton => {
            trigger_export_profile(app);
        }
        Hit::ImportProfileButton => {
            trigger_import_profile(app);
        }
        Hit::VerifyIntegrityButton => {
            trigger_verify_integrity(app);
        }
        Hit::CategoryChip(i) => {
            let name = app
                .visible_categories()
                .get(i)
                .map(|c| c.name.clone());
            if let Some(name) = name {
                app.toggle_category(&name);
                trigger_mod_search(app, false);
            }
        }
        Hit::OpenFiltersButton => {
            app.filters_popup_open = true;
            app.filters_scroll = 0;
        }
        Hit::CloseFiltersPopup => {
            app.filters_popup_open = false;
        }
        Hit::ClearAllFilters => {
            if !app.selected_categories.is_empty() {
                app.selected_categories.clear();
                trigger_mod_search(app, false);
            }
        }
        Hit::CheckUpdatesButton => {
            app.update_modal_dismissed = false;
            update::spawn_check(app.client.clone(), app.worker_tx.clone());
        }
        Hit::RamDecrease => {
            let next = app.max_ram_mb.saturating_sub(512).max(512);
            if next != app.max_ram_mb {
                app.max_ram_mb = next;
                config::save_max_ram(next);
            }
        }
        Hit::RamIncrease => {
            let next = (app.max_ram_mb + 512).min(32768);
            app.max_ram_mb = next;
            config::save_max_ram(next);
        }
        Hit::JavaPathField => {
            app.focus = Focus::JavaPath;
        }
        Hit::ClearJavaPath => {
            app.java_path_input.clear();
            config::save_java_path("");
        }
        Hit::JavaPathForVersionField => {
            app.focus = Focus::JavaPathForVersion;
            // Lock the field to whatever version is currently selected so the
            // user's edit doesn't drift onto another row.
            app.java_path_for_version_id = app
                .selected_modded_id()
                .or_else(|| app.selected_version.clone());
            if let Some(id) = &app.java_path_for_version_id {
                app.java_path_for_version_input = app
                    .java_path_per_version
                    .get(id)
                    .cloned()
                    .unwrap_or_default();
            }
        }
        Hit::ClearJavaPathForVersion => {
            if let Some(id) = app.java_path_for_version_id.clone() {
                app.java_path_per_version.remove(&id);
                config::save_java_path_for(&id, "");
                app.java_path_for_version_input.clear();
            }
        }
        Hit::OpenDataFolder => {
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("explorer.exe")
                    .arg(app.paths.root.as_os_str())
                    .spawn();
            }
            #[cfg(not(windows))]
            {
                let _ = webbrowser::open(&app.paths.root.display().to_string());
            }
            app.status_message = format!("Opened {}", app.paths.root.display());
        }
        Hit::InstallUpdateNow => {
            trigger_install_update(app);
        }
        Hit::DismissUpdate => {
            app.update_modal_dismissed = true;
        }
        Hit::OpenReleasesPage => {
            let url = match &app.update_status {
                app::UpdateStatus::Outdated(info) if !info.html_url.is_empty() => {
                    info.html_url.clone()
                }
                _ => "https://github.com/tinuxongit/tinux-launcher/releases".to_string(),
            };
            let _ = webbrowser::open(&url);
            app.status_message = "Opened releases page".into();
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
        Hit::NewsScrollbar | Hit::ArticleScrollbar | Hit::VersionsScrollbar | Hit::LogsScrollbar => {}
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
        Hit::RotateSkinLeft => app.skin_view = app.skin_view.prev(),
        Hit::RotateSkinRight => app.skin_view = app.skin_view.next(),
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
    if app.filter == app::VersionFilter::Modded {
        trigger_install_fabric(app);
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

fn trigger_install_fabric(app: &mut App) {
    let Some(mc) = app.selected_version.clone() else {
        app.status_message = "Pick a Minecraft version first".into();
        return;
    };
    let Some(loader) = app.latest_stable_fabric_loader().map(|s| s.to_string()) else {
        app.status_message = "Fabric loader list still loading — try again in a moment".into();
        return;
    };
    let Some(manifest) = app.manifest.clone() else {
        app.status_message = "Version manifest not loaded yet".into();
        return;
    };
    let client = app.client.clone();
    let paths_clone = clone_paths(&app.paths);
    let tx = app.worker_tx.clone();
    app.status_message = format!("Installing Fabric {loader} for {mc}...");
    tokio::spawn(async move {
        worker::do_install_fabric(client, paths_clone, manifest, mc, loader, tx).await;
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
    let Some(java) = app.java.clone() else {
        app.status_message = "Java not detected — install Java 17+ and restart".into();
        return;
    };
    let base_opts = match app.account_mode {
        AccountMode::Online => match &app.account {
            Some(a) => launch::LaunchOptions::from_account(a),
            None => {
                app.status_message = "Sign in first, or switch to Offline mode".into();
                return;
            }
        },
        AccountMode::Offline => launch::LaunchOptions::offline(app.offline_name.clone()),
    };
    let target_id = if app.filter == app::VersionFilter::Modded {
        app.selected_modded_id()
            .unwrap_or_else(|| app.selected_version.clone().unwrap_or_default())
    } else {
        app.selected_version.clone().unwrap_or_default()
    };
    let java_override = app
        .java_path_override_for(&target_id)
        .map(std::path::PathBuf::from);
    let opts = base_opts
        .with_max_ram(app.max_ram_mb)
        .with_java_override(java_override);

    let client = app.client.clone();
    let paths_clone = clone_paths(&app.paths);
    let tx = app.worker_tx.clone();

    if app.filter == app::VersionFilter::Modded {
        let Some(mc) = app.selected_version.clone() else {
            app.status_message = "Pick a Minecraft version first".into();
            return;
        };
        let Some(loader) = app.latest_stable_fabric_loader().map(|s| s.to_string()) else {
            app.status_message = "Fabric loader list still loading — try again".into();
            return;
        };
        let Some(manifest) = app.manifest.clone() else {
            app.status_message = "Version manifest not loaded yet".into();
            return;
        };
        config::save_last_played(&mc, app.filter.as_str());
        tokio::spawn(async move {
            worker::do_install_and_launch_fabric(
                client, paths_clone, manifest, mc, loader, java, opts, tx,
            )
            .await;
        });
        return;
    }

    let Some(entry) = app.selected_manifest_entry() else {
        app.status_message = "Pick a version first (Versions tab)".into();
        return;
    };
    config::save_last_played(&entry.id, app.filter.as_str());
    tokio::spawn(async move {
        worker::do_install_and_launch(client, paths_clone, entry, java, opts, tx).await;
    });
}

fn trigger_install_update(app: &mut App) {
    let new_exe = match &app.update_status {
        app::UpdateStatus::Ready { new_exe, .. } => new_exe.clone(),
        _ => {
            app.status_message = "No downloaded update is ready yet".into();
            return;
        }
    };
    match update::spawn_swap_and_restart(&new_exe) {
        Ok(()) => {
            app.status_message = "Restarting to apply update...".into();
            app.running = false;
        }
        Err(e) => {
            app.status_message = format!("Couldn't start updater: {e}");
        }
    }
}

fn trigger_mod_search(app: &mut App, append: bool) {
    let Some(mc) = app.selected_version.clone() else {
        app.status_message = "Pick a Minecraft version first".into();
        return;
    };
    let query = app.mod_search_query.trim().to_string();
    let loader = app.loader.modrinth_key().to_string();
    let kind = app.browser_kind;
    let project_type = kind.project_type().to_string();
    let include_loader = kind.uses_loader();
    let offset = if append { app.mod_search_api_offset } else { 0 };
    let categories = app.selected_categories.clone();
    let client = app.client.clone();
    let tx = app.worker_tx.clone();
    app.mod_search_request_id = app.mod_search_request_id.wrapping_add(1);
    let request_id = app.mod_search_request_id;
    let _ = tx.send(event::WorkerMsg::ModSearchStarted);
    tokio::spawn(async move {
        match modrinth::search(
            &client,
            &query,
            &mc,
            &loader,
            &project_type,
            include_loader,
            &categories,
            offset,
        )
        .await
        {
            Ok(resp) => {
                let _ = tx.send(event::WorkerMsg::ModSearchDone {
                    request_id,
                    hits: resp.hits,
                    total: resp.total_hits,
                    offset,
                    append,
                });
            }
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::ModSearchFailed {
                    request_id,
                    error: format!("{e:#}"),
                });
            }
        }
    });
}

fn trigger_mod_install(app: &mut App, idx: usize) {
    let Some(hit) = app.mod_search_results.get(idx).cloned() else {
        return;
    };
    if app.is_project_installed(&hit.project_id) {
        app.status_message = format!("Already installed: {}", hit.title);
        return;
    }
    let Some(mc) = app.selected_version.clone() else {
        app.status_message = "Pick a Minecraft version first".into();
        return;
    };
    let kind = app.browser_kind;
    let Some(dest_dir) = app.current_content_dir(kind) else {
        app.status_message = "Install the Fabric version first".into();
        return;
    };
    if app.mod_installing.is_some() {
        app.status_message = "An install is already running".into();
        return;
    }
    let loader = if kind.uses_loader() {
        Some(app.loader.modrinth_key().to_string())
    } else {
        None
    };
    let project_id = hit.project_id.clone();
    let client = app.client.clone();
    let tx = app.worker_tx.clone();
    let _ = tx.send(event::WorkerMsg::ModInstallStarted(project_id.clone()));
    app.status_message = format!("Installing: {}", hit.title);
    tokio::spawn(async move {
        let loader_ref = loader.as_deref();
        match modrinth::install_with_deps(&client, &project_id, &mc, loader_ref, &dest_dir).await {
            Ok(result) => {
                let _ = tx.send(event::WorkerMsg::ModInstallDone {
                    project: project_id,
                    filename: result.primary_filename,
                    dep_count: result.dep_project_ids.len(),
                });
            }
            Err(e) => {
                let _ = tx.send(event::WorkerMsg::ModInstallFailed {
                    project: project_id,
                    error: format!("{e:#}"),
                });
            }
        }
    });
}

fn trigger_mod_remove(app: &mut App, idx: usize) {
    let Some(filename) = app.installed_mods.get(idx).cloned() else {
        return;
    };
    let Some(dest_dir) = app.current_content_dir(app.browser_kind) else {
        return;
    };
    let kind = app.browser_kind;
    app.installed_meta.remove_by_filename(kind, &filename);
    app.save_meta();
    let client_dir = dest_dir.clone();
    let tx = app.worker_tx.clone();
    let name_clone = filename.clone();
    tokio::spawn(async move {
        let _ = modrinth::delete(&client_dir, &name_clone).await;
        let _ = tx.send(event::WorkerMsg::ModInstallDone {
            project: String::new(),
            filename: format!("(removed) {name_clone}"),
            dep_count: 0,
        });
    });
    app.status_message = format!("Removing {filename}...");
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

fn trigger_export_profile(app: &mut App) {
    let Some(_id) = app.selected_modded_id() else {
        app.status_message = "Open a modded version to export its profile".into();
        return;
    };
    let dir = app.paths.root.join("profiles");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app.status_message = format!("Couldn't create profiles dir: {e}");
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("profile-{stamp}.json"));
    match serde_json::to_vec_pretty(&app.installed_meta) {
        Ok(bytes) => match std::fs::write(&path, &bytes) {
            Ok(()) => {
                app.status_message = format!("Exported profile to {}", path.display());
            }
            Err(e) => app.status_message = format!("Export failed: {e}"),
        },
        Err(e) => app.status_message = format!("Export serialize failed: {e}"),
    }
}

fn trigger_import_profile(app: &mut App) {
    let Some(_id) = app.selected_modded_id() else {
        app.status_message = "Open a modded version to import a profile into it".into();
        return;
    };
    let Some(mc) = app.selected_version.clone() else { return };
    let path = app.paths.root.join("profiles").join("import.json");
    if !path.exists() {
        app.status_message = format!(
            "Drop a profile file at {} and click Import again",
            path.display()
        );
        return;
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            app.status_message = format!("Couldn't read import.json: {e}");
            return;
        }
    };
    let parsed: meta::InstanceMeta = match serde_json::from_slice(&bytes) {
        Ok(p) => p,
        Err(e) => {
            app.status_message = format!("import.json is malformed: {e}");
            return;
        }
    };
    // Install every project listed in the imported profile that isn't already
    // installed locally. We fire one worker per project.
    let loader = if app.browser_kind.uses_loader() {
        Some(app.loader.modrinth_key().to_string())
    } else {
        None
    };
    let Some(dest_dir) = app.current_content_dir(app.browser_kind) else {
        return;
    };
    let mut queued = 0usize;
    for (project_id, _filename) in parsed.map(app.browser_kind).iter() {
        if app.installed_meta.is_installed(app.browser_kind, project_id) {
            continue;
        }
        let pid = project_id.clone();
        let mc = mc.clone();
        let loader = loader.clone();
        let dest_dir = dest_dir.clone();
        let client = app.client.clone();
        let tx = app.worker_tx.clone();
        let _ = tx.send(event::WorkerMsg::ModInstallStarted(pid.clone()));
        tokio::spawn(async move {
            let loader_ref = loader.as_deref();
            match modrinth::install_with_deps(&client, &pid, &mc, loader_ref, &dest_dir).await {
                Ok(r) => {
                    let _ = tx.send(event::WorkerMsg::ModInstallDone {
                        project: pid,
                        filename: r.primary_filename,
                        dep_count: r.dep_project_ids.len(),
                    });
                }
                Err(e) => {
                    let _ = tx.send(event::WorkerMsg::ModInstallFailed {
                        project: pid,
                        error: format!("{e:#}"),
                    });
                }
            }
        });
        queued += 1;
    }
    app.status_message = format!("Importing {queued} project(s) — watch the installed list");
}

fn trigger_verify_integrity(app: &mut App) {
    if app.integrity_in_progress {
        app.status_message = "Verification already running".into();
        return;
    }
    let Some(entry) = app.selected_manifest_entry() else {
        app.status_message = "Pick a version to verify".into();
        return;
    };
    app.integrity_in_progress = true;
    app.status_message = format!("Verifying {}...", entry.id);
    let client = app.client.clone();
    let paths_clone = clone_paths(&app.paths);
    let tx = app.worker_tx.clone();
    tokio::spawn(async move {
        worker::do_verify_integrity(client, paths_clone, entry, tx).await;
    });
}

fn switch_browser_tab(app: &mut App, kind: app::ContentKind) {
    app.browser_kind = kind;
    app.mod_search_query.clear();
    app.mod_search_results.clear();
    app.mod_search_offset = 0;
    app.mod_search_api_offset = 0;
    app.mod_search_total = 0;
    app.selected_categories.clear();
    app.reload_meta();
    app.refresh_installed_mods();
    trigger_mod_search(app, false);
}

fn update_modal_visible(app: &App) -> bool {
    if app.update_modal_dismissed {
        return false;
    }
    matches!(
        app.update_status,
        app::UpdateStatus::Downloading { .. } | app::UpdateStatus::Ready { .. }
    )
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
        vanilla_minecraft: p.vanilla_minecraft.clone(),
        logs: p.logs.clone(),
        cache: p.cache.clone(),
    }
}
