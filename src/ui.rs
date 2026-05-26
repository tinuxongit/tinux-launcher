use crate::app::{App, Focus, InstallState, LaunchState, VersionFilter};
use crate::event::{Hit, Tab};
use crate::theme;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Widget, Wrap,
    },
    Frame,
};

const HEADER_HEIGHT: u16 = 3;
const STATUS_HEIGHT: u16 = 1;

struct Fill {
    style: Style,
}

impl Widget for Fill {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let cell = &mut buf[(x, y)];
                cell.reset();
                cell.set_style(self.style);
            }
        }
    }
}

fn wipe(f: &mut Frame, area: Rect) {
    f.render_widget(
        Fill {
            style: theme::base(),
        },
        area,
    );
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.click_regions.clear();
    let size = f.area();
    app.last_size = size;

    wipe(f, size);

    let frame = size.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_HEIGHT),
            Constraint::Length(1), // gap
            Constraint::Min(0),
            Constraint::Length(1), // gap
            Constraint::Length(STATUS_HEIGHT),
        ])
        .split(frame);

    draw_header(f, app, outer[0]);
    match app.tab {
        Tab::Play => draw_play(f, app, outer[2]),
        Tab::Versions => draw_versions(f, app, outer[2]),
        Tab::Accounts => draw_accounts(f, app, outer[2]),
        Tab::Logs => draw_logs(f, app, outer[2]),
    }
    draw_status(f, app, outer[4]);
}

fn draw_header(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title + tabs
            Constraint::Length(1), // separator
            Constraint::Length(1), // bottom spacing inside header
        ])
        .split(area);

    let title = Span::styled(
        " ⛏  Tinux Launcher",
        Style::default()
            .fg(theme::ACCENT_HI)
            .add_modifier(Modifier::BOLD),
    );
    let title_w = title.width() as u16;
    f.render_widget(
        Paragraph::new(Line::from(vec![title])).style(theme::base()),
        Rect::new(rows[0].x, rows[0].y, title_w.min(rows[0].width), 1),
    );

    let mut x = rows[0].x + title_w + 3;
    for tab in Tab::ALL {
        let label = format!(" {} ", tab.label());
        let w = label.chars().count() as u16;
        if x + w > rows[0].x + rows[0].width {
            break;
        }
        let rect = Rect::new(x, rows[0].y, w, 1);
        let active = app.tab == tab;
        let hovered = app.hover == Some(Hit::Tab(tab));
        let style = if active {
            theme::button_hover()
        } else if hovered {
            theme::button_idle().add_modifier(Modifier::BOLD)
        } else {
            theme::button_idle()
        };
        f.render_widget(Paragraph::new(label).style(style), rect);
        app.click_regions.push((rect, Hit::Tab(tab)));
        x += w + 1;
    }

    let sep: String = "─".repeat(rows[1].width as usize);
    f.render_widget(
        Paragraph::new(sep).style(Style::default().fg(theme::BORDER).bg(theme::BG)),
        rows[1],
    );
}

fn draw_play(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Play ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0 Selected: ...
            Constraint::Length(1), // 1 gap
            Constraint::Length(1), // 2 buttons
            Constraint::Length(1), // 3 gap
            Constraint::Length(1), // 4 Account: ...
            Constraint::Length(1), // 5 gap
            Constraint::Length(1), // 6 offline row
            Constraint::Length(1), // 7 gap
            Constraint::Length(2), // 8 progress (bar + label)
            Constraint::Min(0),
        ])
        .split(inner);

    let sel = app
        .selected_manifest_entry()
        .map(|v| format!("{} ({})", v.id, v.kind.label()))
        .unwrap_or_else(|| "(no version selected — open the Versions tab)".to_string());
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Selected: ", theme::dim()),
            Span::styled(sel, theme::accent_bold()),
        ]))
        .style(theme::base()),
        rows[0],
    );

    let buttons = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(14),
            Constraint::Length(2),
            Constraint::Length(14),
            Constraint::Min(0),
        ])
        .split(rows[2]);
    draw_button(f, app, buttons[0], "▶ Launch", Hit::LaunchButton, true);
    draw_button(f, app, buttons[2], "⬇ Install", Hit::InstallButton, false);

    let acct_line = if let Some(a) = &app.account {
        Line::from(vec![
            Span::styled("Account: ", theme::dim()),
            Span::styled(
                format!("● {} (Microsoft)", a.username),
                Style::default().fg(theme::ACCENT_HI),
            ),
        ])
    } else if app.auth_in_progress {
        Line::from(Span::styled(
            "Account: signing in...",
            Style::default().fg(theme::GOLD),
        ))
    } else {
        Line::from(vec![
            Span::styled("Account: ", theme::dim()),
            Span::styled("not signed in", Style::default().fg(theme::FG_DIM)),
        ])
    };
    f.render_widget(Paragraph::new(acct_line).style(theme::base()), rows[4]);

    let offline_row = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),
            Constraint::Length(22),
            Constraint::Length(2),
            Constraint::Length(16),
            Constraint::Min(0),
        ])
        .split(rows[6]);
    f.render_widget(
        Paragraph::new("Offline name:").style(theme::dim()),
        offline_row[0],
    );
    draw_offline_name(f, app, offline_row[1]);
    if app.account.is_some() {
        draw_button(f, app, offline_row[3], "Sign out", Hit::LogoutButton, false);
    } else {
        draw_button(f, app, offline_row[3], "Sign in (MS)", Hit::LoginButton, false);
    }

    draw_progress(f, app, rows[8]);
}

fn draw_offline_name(f: &mut Frame, app: &mut App, rect: Rect) {
    let focused = app.focus == Focus::OfflineName;
    let mut content = format!(" {}", app.offline_name);
    if focused {
        content.push('▎');
    } else {
        content.push(' ');
    }
    while content.chars().count() < rect.width as usize {
        content.push(' ');
    }

    let style = if focused {
        Style::default()
            .fg(theme::ACCENT_HI)
            .bg(theme::PANEL)
            .add_modifier(Modifier::UNDERLINED)
    } else if app.hover == Some(Hit::OfflineNameField) {
        Style::default()
            .fg(theme::FG)
            .bg(theme::PANEL_HI)
            .add_modifier(Modifier::UNDERLINED)
    } else {
        Style::default()
            .fg(theme::FG)
            .bg(theme::PANEL_HI)
            .add_modifier(Modifier::UNDERLINED)
    };
    f.render_widget(Paragraph::new(content).style(style), rect);
    app.click_regions.push((rect, Hit::OfflineNameField));
}

fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let Some(state) = &app.install else {
        let hint = if app.launch_state == LaunchState::Running {
            "Minecraft is running..."
        } else {
            ""
        };
        f.render_widget(Paragraph::new(hint).style(theme::dim()), area);
        return;
    };
    let InstallState { done, total, what, .. } = state;
    let pct = if *total == 0 {
        0.0
    } else {
        (*done as f64 / *total as f64).clamp(0.0, 1.0)
    };
    let width = area.width.saturating_sub(2) as usize;
    let filled = (pct * width as f64).round() as usize;
    let bar: String = std::iter::repeat('█')
        .take(filled)
        .chain(std::iter::repeat('░').take(width.saturating_sub(filled)))
        .collect();
    let line1 = Line::from(vec![
        Span::styled(bar, Style::default().fg(theme::ACCENT)),
        Span::raw(" "),
        Span::styled(format!("{:.0}%", pct * 100.0), theme::accent_bold()),
    ]);
    let line2 = Line::from(vec![Span::styled(what.clone(), theme::dim())]);
    f.render_widget(
        Paragraph::new(vec![line1, line2]).style(theme::base()),
        area,
    );
}

fn draw_versions(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Versions ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    let filters = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Min(0),
        ])
        .split(rows[0]);
    draw_filter(
        f,
        app,
        filters[0],
        " Releases ",
        Hit::FilterReleases,
        VersionFilter::Releases,
    );
    draw_filter(
        f,
        app,
        filters[1],
        " Snapshots ",
        Hit::FilterSnapshots,
        VersionFilter::Snapshots,
    );
    draw_filter(
        f,
        app,
        filters[2],
        "  Older  ",
        Hit::FilterOld,
        VersionFilter::Old,
    );

    let list_area = rows[1];
    let list_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(list_area);
    let content_rect = list_chunks[0];
    let sb_rect = list_chunks[1];

    wipe(f, content_rect);

    let rows_n = content_rect.height as usize;
    let snapshot: Vec<(String, String, String)> = app
        .visible_versions()
        .iter()
        .map(|v| {
            (
                v.id.clone(),
                v.kind.label().to_string(),
                v.release_time
                    .split('T')
                    .next()
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect();
    let total = snapshot.len();
    if total > rows_n && app.list_offset + rows_n > total {
        app.list_offset = total - rows_n;
    }
    if total <= rows_n {
        app.list_offset = 0;
    }
    let start = app.list_offset;
    let end = (start + rows_n).min(total);

    for (i, (id, kind_label, date)) in snapshot[start..end].iter().enumerate() {
        let global_idx = start + i;
        let y = content_rect.y + i as u16;
        let rect = Rect::new(content_rect.x, y, content_rect.width, 1);
        let selected = app.selected_version.as_deref() == Some(id.as_str());
        let hovered = app.hover == Some(Hit::VersionRow(global_idx));
        let style = if selected {
            theme::list_selected()
        } else if hovered {
            theme::button_idle()
        } else {
            theme::base()
        };
        let marker = if selected { "▶" } else { " " };
        let line = format!(" {marker} {:<18} {:<10} {date} ", id, kind_label);
        f.render_widget(Paragraph::new(line).style(style), rect);
        app.click_regions.push((rect, Hit::VersionRow(global_idx)));
    }

    if total == 0 {
        let msg = if app.manifest.is_none() {
            "Loading version manifest..."
        } else {
            "No versions in this filter."
        };
        f.render_widget(Paragraph::new(msg).style(theme::dim()), content_rect);
    } else if total > rows_n {
        let mut sb_state = ScrollbarState::new(total.saturating_sub(rows_n))
            .position(app.list_offset)
            .viewport_content_length(rows_n);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme::BORDER).bg(theme::BG))
            .thumb_style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(sb, sb_rect, &mut sb_state);
    }
}

fn draw_filter(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    label: &str,
    hit: Hit,
    filter: VersionFilter,
) {
    let active = app.filter == filter;
    let style = if active {
        theme::button_hover()
    } else if app.hover == Some(hit) {
        theme::button_idle().add_modifier(Modifier::BOLD)
    } else {
        theme::button_idle()
    };
    f.render_widget(Paragraph::new(label).style(style), rect);
    app.click_regions.push((rect, hit));
}

fn draw_accounts(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Accounts ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    f.render_widget(block, area);

    let signed_in = app.account.is_some();
    let has_client = crate::auth::client_id().is_some();

    let mut y = inner.y;
    let row = |yy: u16| Rect::new(inner.x, yy, inner.width, 1);

    let ms_line = match &app.account {
        Some(a) => Line::from(vec![
            Span::styled("Microsoft: ", theme::dim()),
            Span::styled(
                format!("● {} ({})", a.username, a.uuid),
                Style::default().fg(theme::ACCENT_HI),
            ),
        ]),
        None => Line::from(vec![
            Span::styled("Microsoft: ", theme::dim()),
            Span::styled("not signed in", Style::default().fg(theme::FG_DIM)),
        ]),
    };
    f.render_widget(Paragraph::new(ms_line).style(theme::base()), row(y));
    y += 2;

    if !signed_in && !has_client {
        let header = Span::styled(
            "Sign-in needs your own Azure app (free, ~5 min):",
            Style::default()
                .fg(theme::GOLD)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(Paragraph::new(Line::from(header)).style(theme::base()), row(y));
        y += 1;
        let steps = [
            "  1. portal.azure.com  →  App registrations  →  New registration",
            "  2. Supported accounts: Personal Microsoft accounts only",
            "  3. Add platform → Mobile/desktop → http://localhost/callback",
            "  4. Copy the Application (client) ID from the overview page",
            "  5. Paste it as \"ms_client_id\" in the config file below",
        ];
        for s in steps {
            f.render_widget(
                Paragraph::new(s).style(theme::dim()),
                row(y),
            );
            y += 1;
        }
        y += 1;

        let cfg_label = match crate::config::path() {
            Some(p) => p.display().to_string(),
            None => "(could not resolve config path)".into(),
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Config: ", theme::dim()),
                Span::styled(cfg_label, Style::default().fg(theme::ACCENT)),
            ]))
            .style(theme::base()),
            row(y),
        );
        y += 2;
    } else if has_client && !signed_in {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Client ID configured. Ready to sign in.",
                Style::default().fg(theme::ACCENT),
            )))
            .style(theme::base()),
            row(y),
        );
        y += 2;
    }

    if let Some(e) = &app.auth_error {
        let err_height = inner.y + inner.height - y;
        let err_rect = Rect::new(inner.x, y, inner.width, err_height.min(3));
        f.render_widget(
            Paragraph::new(format!("Last error: {e}"))
                .style(Style::default().fg(theme::RED).bg(theme::BG))
                .wrap(Wrap { trim: true }),
            err_rect,
        );
        y += err_rect.height + 1;
    }

    let btn_row_rect = row(y);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),
            Constraint::Length(2),
            Constraint::Length(15),
            Constraint::Min(0),
        ])
        .split(btn_row_rect);
    if signed_in {
        draw_button(f, app, cols[0], "Sign out", Hit::LogoutButton, false);
    } else {
        draw_button(f, app, cols[0], "Sign in (MS)", Hit::LoginButton, has_client);
    }
    draw_button(f, app, cols[2], "Open config", Hit::OpenConfigButton, false);
}

fn draw_logs(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Logs ", theme::accent_bold()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // toolbar
            Constraint::Length(1), // gap
            Constraint::Min(0),    // log content
        ])
        .split(inner.inner(Margin {
            horizontal: 1,
            vertical: 0,
        }));

    let toolbar = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),
            Constraint::Length(2),
            Constraint::Length(11),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(body[0]);
    draw_button(f, app, toolbar[0], "Copy selection", Hit::CopyLineButton, false);
    draw_button(f, app, toolbar[2], "Copy all", Hit::CopyAllButton, false);
    let hint = match app.selected_log_range {
        Some((a, b)) => {
            let n = (a.max(b) - a.min(b)) + 1;
            let s = if n == 1 { "" } else { "s" };
            format!("{n} line{s} selected — Ctrl+click to extend, Ctrl+C to copy")
        }
        None => "Click a line to select  •  Ctrl+click extends  •  Ctrl+A all  •  Ctrl+C copies".into(),
    };
    f.render_widget(Paragraph::new(hint).style(theme::dim()), toolbar[4]);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(body[2]);
    let content = chunks[0];
    let sb_area = chunks[1];

    wipe(f, content);

    let n = app.logs.len();
    let h = content.height as usize;
    if h == 0 {
        return;
    }

    if n > h {
        let max_off = n - h;
        if app.log_offset > max_off {
            app.log_offset = max_off;
        }
    } else {
        app.log_offset = 0;
    }
    let end = n.saturating_sub(app.log_offset);
    let start = end.saturating_sub(h);

    let sel_range = app.selected_log_range.map(|(a, b)| (a.min(b), a.max(b)));
    for (i, line) in app.logs.iter().skip(start).take(end - start).enumerate() {
        let row_y = content.y + i as u16;
        let row_rect = Rect::new(content.x, row_y, content.width, 1);
        let global_idx = start + i;
        let selected = matches!(sel_range, Some((lo, hi)) if global_idx >= lo && global_idx <= hi);
        let style = if selected {
            theme::list_selected()
        } else {
            theme::base()
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::raw(line.clone()))).style(style),
            row_rect,
        );
        app.click_regions.push((row_rect, Hit::LogRow(global_idx)));
    }

    if n > h {
        let scroll_range = n - h;
        let pos = scroll_range.saturating_sub(app.log_offset);
        let mut sb_state = ScrollbarState::new(scroll_range)
            .position(pos)
            .viewport_content_length(h);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme::BORDER).bg(theme::BG))
            .thumb_style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(sb, sb_area, &mut sb_state);
    }
}

fn draw_button(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    label: &str,
    hit: Hit,
    primary: bool,
) {
    let hovered = app.hover == Some(hit);
    let style = if primary && hovered {
        theme::button_primary().add_modifier(Modifier::REVERSED)
    } else if primary {
        theme::button_primary()
    } else if hovered {
        theme::button_hover()
    } else {
        theme::button_idle()
    };
    f.render_widget(
        Paragraph::new(label)
            .style(style)
            .alignment(Alignment::Center),
        rect,
    );
    app.click_regions.push((rect, hit));
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let java = match &app.java {
        Some(j) => format!("Java {}", j.major),
        None => "no Java".into(),
    };
    let acct = match &app.account {
        Some(a) => format!("● {}", a.username),
        None => "● offline".into(),
    };
    let line = Line::from(vec![
        Span::styled(" ", theme::base()),
        Span::styled(acct, Style::default().fg(theme::ACCENT)),
        Span::styled("  •  ", theme::dim()),
        Span::styled(java, theme::dim()),
        Span::styled("  •  ", theme::dim()),
        Span::styled(app.status_message.clone(), theme::dim()),
    ]);
    f.render_widget(Paragraph::new(line).style(theme::base()), area);
}

pub fn hit_test(app: &App, col: u16, row: u16) -> Option<Hit> {
    for (rect, hit) in app.click_regions.iter().rev() {
        if col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
        {
            return Some(*hit);
        }
    }
    None
}
