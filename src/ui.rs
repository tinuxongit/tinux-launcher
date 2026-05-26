use crate::app::{AccountMode, App, Focus, InstallState, LaunchState, VersionFilter};
use crate::event::{Hit, Tab};
use crate::theme;
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Widget, Wrap,
    },
    Frame,
};

const HEADER_HEIGHT: u16 = 4;
const STATUS_HEIGHT: u16 = 1;
const BUTTON_H: u16 = 3;

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
            Constraint::Length(1), // title + version
            Constraint::Length(1), // gap
            Constraint::Length(1), // tabs
            Constraint::Length(1), // separator + active-tab underline
        ])
        .split(area);

    let title = Line::from(vec![
        Span::styled(
            " ⛏  Tinux Launcher",
            Style::default()
                .fg(theme::ACCENT_HI)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  v{}", env!("CARGO_PKG_VERSION")),
            theme::dim(),
        ),
    ]);
    f.render_widget(Paragraph::new(title).style(theme::base()), rows[0]);

    let mut x = rows[2].x + 1;
    let mut active_seg: Option<(u16, u16)> = None;
    for tab in Tab::ALL {
        let label = format!(" {} ", tab.label());
        let w = label.chars().count() as u16;
        if x + w > rows[2].x + rows[2].width {
            break;
        }
        let rect = Rect::new(x, rows[2].y, w, 1);
        let active = app.tab == tab;
        let hovered = app.hover == Some(Hit::Tab(tab));
        let style = if active {
            Style::default()
                .fg(theme::ACCENT)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD)
        } else if hovered {
            Style::default().fg(theme::FG).bg(theme::BG)
        } else {
            Style::default().fg(theme::FG_DIM).bg(theme::BG)
        };
        f.render_widget(Paragraph::new(label).style(style), rect);
        app.click_regions.push((rect, Hit::Tab(tab)));
        if active {
            active_seg = Some((x, w));
        }
        x += w + 2;
    }

    let sep: String = "─".repeat(rows[3].width as usize);
    f.render_widget(
        Paragraph::new(sep).style(Style::default().fg(theme::BORDER).bg(theme::BG)),
        rows[3],
    );
    if let Some((ax, aw)) = active_seg {
        let overlay = "━".repeat(aw as usize);
        f.render_widget(
            Paragraph::new(overlay).style(Style::default().fg(theme::ACCENT).bg(theme::BG)),
            Rect::new(ax, rows[3].y, aw, 1),
        );
    }
}

fn draw_play(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Play ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),        // 0 Selected: ...
            Constraint::Length(1),        // 1 Playing as: ...
            Constraint::Length(1),        // 2 gap
            Constraint::Length(BUTTON_H), // 3 button
            Constraint::Length(1),        // 4 gap
            Constraint::Length(2),        // 5 progress
            Constraint::Length(1),        // 6 gap
            Constraint::Length(1),        // 7 news header
            Constraint::Min(0),           // 8 news list
        ])
        .split(inner);

    let sel = app
        .selected_manifest_entry()
        .map(|v| format!("{} ({})", v.id, v.kind.label()))
        .unwrap_or_else(|| "(no version selected)".to_string());
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Selected: ", theme::dim()),
            Span::styled(sel, theme::accent_bold()),
        ]))
        .style(theme::base()),
        rows[0],
    );

    let as_line = match (app.account_mode, &app.account) {
        (AccountMode::Online, Some(a)) => Line::from(vec![
            Span::styled("Playing as: ", theme::dim()),
            Span::styled(
                format!("● {} (Microsoft)", a.username),
                Style::default().fg(theme::ACCENT_HI),
            ),
        ]),
        (AccountMode::Online, None) => Line::from(vec![
            Span::styled("Playing as: ", theme::dim()),
            Span::styled(
                "(online mode — not signed in)",
                Style::default().fg(theme::GOLD),
            ),
        ]),
        (AccountMode::Offline, _) => Line::from(vec![
            Span::styled("Playing as: ", theme::dim()),
            Span::styled(
                format!("{} (offline)", app.offline_name),
                Style::default().fg(theme::FG),
            ),
        ]),
    };
    f.render_widget(Paragraph::new(as_line).style(theme::base()), rows[1]);

    let btn_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(22), Constraint::Min(0)])
        .split(rows[3]);
    let installed = app.selected_is_installed();
    if installed {
        draw_button(f, app, btn_cols[0], "▶  Launch", Hit::LaunchButton, true);
    } else {
        draw_button(f, app, btn_cols[0], "⬇  Install", Hit::InstallButton, true);
    }

    draw_progress(f, app, rows[5]);
    draw_news_header(f, rows[7]);
    draw_news_list(f, app, rows[8]);
}

fn draw_news_header(f: &mut Frame, area: Rect) {
    let title = " Latest news ";
    let mut s = String::new();
    s.push_str(title);
    while s.chars().count() < area.width as usize {
        s.push('─');
    }
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(title, theme::accent_bold()),
            Span::styled(
                "─".repeat(area.width as usize - title.chars().count()),
                Style::default().fg(theme::BORDER).bg(theme::BG),
            ),
        ]))
        .style(theme::base()),
        area,
    );
}

fn draw_news_list(f: &mut Frame, app: &mut App, area: Rect) {
    if app.news.is_empty() {
        f.render_widget(
            Paragraph::new("Loading news from minecraft.net...").style(theme::dim()),
            area,
        );
        return;
    }
    let visible = (area.height as usize).min(app.news.len());
    for (i, entry) in app.news.iter().take(visible).enumerate() {
        let y = area.y + i as u16;
        let rect = Rect::new(area.x, y, area.width, 1);
        let hovered = app.hover == Some(Hit::NewsItem(i));
        let title_style = if hovered {
            Style::default()
                .fg(theme::ACCENT_HI)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme::FG).bg(theme::BG)
        };
        let line = Line::from(vec![
            Span::styled("▸ ", Style::default().fg(theme::ACCENT)),
            Span::styled(format!("{}  ", entry.date), theme::dim()),
            Span::styled(entry.title.clone(), title_style),
            Span::styled(
                if entry.category.is_empty() {
                    String::new()
                } else {
                    format!("  · {}", entry.category)
                },
                theme::dim(),
            ),
        ]);
        f.render_widget(Paragraph::new(line).style(theme::base()), rect);
        app.click_regions.push((rect, Hit::NewsItem(i)));
    }
}

fn draw_vcentered_label(f: &mut Frame, label: &str, rect: Rect, style: Style) {
    let mid = rect.height / 2;
    let lines: Vec<Line> = (0..rect.height)
        .map(|i| if i == mid { Line::from(label) } else { Line::from("") })
        .collect();
    f.render_widget(Paragraph::new(lines).style(style), rect);
}

fn draw_offline_name(f: &mut Frame, app: &mut App, rect: Rect) {
    let focused = app.focus == Focus::OfflineName;
    let hovered = app.hover == Some(Hit::OfflineNameField);
    let border_fg = if focused {
        theme::ACCENT
    } else if hovered {
        theme::FG_DIM
    } else {
        theme::BORDER
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_fg).bg(theme::BG))
        .style(theme::base());
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let content = if focused {
        format!("{}▎", app.offline_name)
    } else {
        app.offline_name.clone()
    };
    draw_vcentered_label(f, &format!(" {content}"), inner, theme::base());

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
        .border_type(BorderType::Rounded)
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
        .constraints([
            Constraint::Length(BUTTON_H),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let filter_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(48), Constraint::Min(0)])
        .split(rows[0]);
    draw_segmented(
        f,
        app,
        filter_cols[0],
        &[
            (
                "Releases",
                Hit::FilterReleases,
                app.filter == VersionFilter::Releases,
            ),
            (
                "Snapshots",
                Hit::FilterSnapshots,
                app.filter == VersionFilter::Snapshots,
            ),
            ("Older", Hit::FilterOld, app.filter == VersionFilter::Old),
        ],
    );

    let list_area = rows[2];
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

fn draw_accounts(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Accounts ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, area);

    let online = app.account_mode == AccountMode::Online;

    // Mode segmented toggle.
    f.render_widget(
        Paragraph::new(Span::styled("Mode:", theme::dim())).style(theme::base()),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let toggle_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(34), Constraint::Min(0)])
        .split(Rect::new(inner.x, inner.y + 1, inner.width, BUTTON_H));
    draw_segmented(
        f,
        app,
        toggle_cols[0],
        &[
            ("Offline", Hit::ModeOffline, !online),
            ("Online", Hit::ModeOnline, online),
        ],
    );

    let mut y = inner.y + 1 + BUTTON_H + 1;
    let row = |yy: u16| Rect::new(inner.x, yy, inner.width, 1);

    if online {
        let ms_line = match &app.account {
            Some(a) => Line::from(vec![
                Span::styled("Microsoft: ", theme::dim()),
                Span::styled(
                    format!("● {} ({})", a.username, a.uuid),
                    Style::default().fg(theme::ACCENT_HI),
                ),
            ]),
            None if app.auth_in_progress => Line::from(Span::styled(
                "Microsoft: signing in...",
                Style::default().fg(theme::GOLD),
            )),
            None => Line::from(vec![
                Span::styled("Microsoft: ", theme::dim()),
                Span::styled("not signed in", Style::default().fg(theme::FG_DIM)),
            ]),
        };
        f.render_widget(Paragraph::new(ms_line).style(theme::base()), row(y));
        y += 2;

        if let Some(e) = &app.auth_error {
            let err_rect = Rect::new(inner.x, y, inner.width, 3);
            f.render_widget(
                Paragraph::new(format!("Last error: {e}"))
                    .style(Style::default().fg(theme::RED).bg(theme::BG))
                    .wrap(Wrap { trim: true }),
                err_rect,
            );
            y += 4;
        }

        let btn_row_rect = Rect::new(inner.x, y, inner.width, BUTTON_H);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(18),
                Constraint::Length(2),
                Constraint::Length(18),
                Constraint::Min(0),
            ])
            .split(btn_row_rect);
        if app.account.is_some() {
            draw_button(f, app, cols[0], "Sign out", Hit::LogoutButton, false);
        } else {
            draw_button(f, app, cols[0], "Sign in (MS)", Hit::LoginButton, true);
        }
        draw_button(f, app, cols[2], "Open config", Hit::OpenConfigButton, false);
    } else {
        let label_row = row(y);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(15), Constraint::Min(0)])
            .split(label_row);
        f.render_widget(
            Paragraph::new("Offline name:").style(theme::dim()),
            cols[0],
        );
        y += 1;

        let field_row = Rect::new(inner.x, y, inner.width, BUTTON_H);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(0)])
            .split(field_row);
        draw_offline_name(f, app, cols[0]);
    }
}

fn draw_segmented(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    items: &[(&str, Hit, bool)],
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER).bg(theme::BG));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let n = items.len() as u16;
    if n == 0 || inner.height == 0 || inner.width == 0 {
        return;
    }
    let mid = inner.height / 2;

    // Vertical dividers between sections.
    for i in 1..n {
        let div_x = inner.x + inner.width * i / n;
        let div_lines: Vec<Line> = (0..inner.height).map(|_| Line::from("│")).collect();
        f.render_widget(
            Paragraph::new(div_lines).style(Style::default().fg(theme::BORDER).bg(theme::BG)),
            Rect::new(div_x, inner.y, 1, inner.height),
        );
    }

    for (i, &(label, hit, active)) in items.iter().enumerate() {
        let i = i as u16;
        let seg_x = inner.x + inner.width * i / n;
        let seg_end = inner.x + inner.width * (i + 1) / n;
        let seg_w = seg_end - seg_x;
        let click_rect = Rect::new(seg_x, inner.y, seg_w, inner.height);

        // Leave a 1-col gutter for the left-side divider (except on the first segment).
        let label_x = if i > 0 { seg_x + 1 } else { seg_x };
        let label_w = if i > 0 { seg_w - 1 } else { seg_w };
        let label_rect = Rect::new(label_x, inner.y, label_w, inner.height);

        let hovered = app.hover == Some(hit);
        let fg = if active {
            theme::ACCENT_HI
        } else if hovered {
            theme::FG
        } else {
            theme::FG_DIM
        };
        let modifier = if active { Modifier::BOLD } else { Modifier::empty() };
        let styled = Span::styled(
            label,
            Style::default().fg(fg).bg(theme::BG).add_modifier(modifier),
        );
        let lines: Vec<Line> = (0..label_rect.height)
            .map(|y| if y == mid { Line::from(vec![styled.clone()]) } else { Line::from("") })
            .collect();
        f.render_widget(
            Paragraph::new(lines)
                .style(theme::base())
                .alignment(Alignment::Center),
            label_rect,
        );

        app.click_regions.push((click_rect, hit));
    }
}

fn draw_logs(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Logs ", theme::accent_bold()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(BUTTON_H), // toolbar
            Constraint::Length(1),        // gap
            Constraint::Min(0),           // log content
        ])
        .split(inner.inner(Margin {
            horizontal: 1,
            vertical: 0,
        }));

    let toolbar = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Length(2),
            Constraint::Length(14),
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
            format!("{n} line{s} selected — Ctrl+click extends, Ctrl+C copies")
        }
        None => "Click a line  •  Ctrl+click extends  •  Ctrl+A all  •  Ctrl+C copies".into(),
    };
    draw_vcentered_label(f, &hint, toolbar[4], theme::dim());

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
    // Single-row variant: flat colored rect, no border.
    if rect.height < 3 {
        let style = if primary {
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
        return;
    }

    let (fg, border_fg) = if primary && hovered {
        (theme::ACCENT_HI, theme::ACCENT_HI)
    } else if primary {
        (theme::GOLD, theme::GOLD)
    } else if hovered {
        (theme::ACCENT, theme::ACCENT)
    } else {
        (theme::FG, theme::BORDER)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_fg).bg(theme::BG));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mid = inner.height / 2;
    let styled = Span::styled(
        label,
        Style::default()
            .fg(fg)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD),
    );
    let lines: Vec<Line> = (0..inner.height)
        .map(|i| if i == mid { Line::from(vec![styled.clone()]) } else { Line::from("") })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .style(theme::base())
            .alignment(Alignment::Center),
        inner,
    );

    app.click_regions.push((rect, hit));
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let java = match &app.java {
        Some(j) => format!("Java {}", j.major),
        None => "no Java".into(),
    };
    let acct = match &app.account {
        Some(a) => a.username.clone(),
        None => "offline".into(),
    };
    let dot = Span::styled("● ", Style::default().fg(theme::ACCENT).bg(theme::BG));
    let sp = Span::styled("   ", theme::base());

    let left = Line::from(vec![
        dot.clone(),
        Span::styled(acct, theme::dim()),
        sp.clone(),
        dot.clone(),
        Span::styled(java, theme::dim()),
        sp,
        dot,
        Span::styled(app.status_message.clone(), theme::dim()),
    ]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(22)])
        .split(area);
    f.render_widget(Paragraph::new(left).style(theme::base()), cols[0]);

    let right = Line::from(vec![
        Span::styled("📦 ", Style::default().fg(theme::ACCENT).bg(theme::BG)),
        Span::styled("Enjoy your game!", theme::dim()),
    ]);
    f.render_widget(
        Paragraph::new(right)
            .style(theme::base())
            .alignment(Alignment::Right),
        cols[1],
    );
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
