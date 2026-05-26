use crate::app::{
    AccountMode, App, ContentKind, Focus, InstallState, LaunchState, ModLoader, SkinModel,
    UpdateStatus, VersionFilter,
};
use crate::modrinth::SearchHit;
use crate::event::{Hit, InstallKind, Tab};
use crate::news::Block as ArticleBlock;
use crate::skin::SkinPreviewWidget;
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
const SKIN_PREVIEW_BOX_H: u16 = 24;

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
        Tab::Profile => draw_accounts(f, app, outer[2]),
        Tab::Logs => draw_logs(f, app, outer[2]),
        Tab::Settings => draw_settings(f, app, outer[2]),
    }
    if app.mod_browser_open {
        draw_mod_browser(f, app, frame);
    }
    if app.mod_browser_open && app.filters_popup_open {
        draw_filters_popup(f, app, frame);
    }
    if update_modal_visible(app) {
        draw_update_modal(f, app, frame);
    }
    if app.info_popup.is_some() {
        draw_info_popup(f, app, frame);
    }
    draw_status(f, app, outer[4]);
}

fn draw_filters_popup(f: &mut Frame, app: &mut App, area: Rect) {
    let w = 96u16.min(area.width.saturating_sub(4));
    let h = 28u16.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);

    f.render_widget(Fill { style: theme::base() }, rect);

    let active = app.selected_categories.len();
    let title = if active == 0 {
        format!(" Filters · {} ", app.browser_kind.label())
    } else {
        format!(" Filters · {} · {} active ", app.browser_kind.label(), active)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .style(theme::base())
        .title(Span::styled(title, theme::accent_bold()));
    let inner = block.inner(rect).inner(Margin {
        horizontal: 3,
        vertical: 1,
    });
    f.render_widget(block, rect);

    // Heading line — what this popup is for.
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "Pick categories to narrow the results",
                Style::default().fg(theme::FG).bg(theme::BG),
            ),
            Span::styled(
                "  ·  multi-select  ·  changes apply live",
                theme::dim(),
            ),
        ]))
        .style(theme::base()),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    // Reserve room for action buttons + 1 padding row.
    let footer_h = BUTTON_H + 1;
    let body_top = inner.y + 2;
    let body_h = inner
        .height
        .saturating_sub(2 + footer_h);
    let body_rect = Rect::new(inner.x, body_top, inner.width, body_h);

    let cats: Vec<crate::modrinth::Category> = app
        .visible_categories()
        .into_iter()
        .cloned()
        .collect();
    if cats.is_empty() {
        f.render_widget(
            Paragraph::new("Loading categories from Modrinth...")
                .style(theme::dim())
                .wrap(Wrap { trim: true }),
            body_rect,
        );
    } else {
        draw_grouped_chips(f, app, body_rect, &cats);
    }

    let btn_y = inner.y + inner.height.saturating_sub(BUTTON_H);
    let btn_row = Rect::new(inner.x, btn_y, inner.width, BUTTON_H);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(16),
            Constraint::Min(0),
            Constraint::Length(12),
        ])
        .split(btn_row);
    let has_active = active > 0;
    if has_active {
        draw_button(f, app, cols[0], "Clear all", Hit::ClearAllFilters, false);
    } else {
        draw_disabled_button(f, cols[0], "Clear all");
    }
    draw_button(f, app, cols[2], "Done", Hit::CloseFiltersPopup, true);
}

fn draw_grouped_chips(
    f: &mut Frame,
    app: &mut App,
    area: Rect,
    cats: &[crate::modrinth::Category],
) {
    // Group categories by their Modrinth `header` value, preserving order.
    let mut groups: Vec<(String, Vec<(usize, &crate::modrinth::Category)>)> = Vec::new();
    for (idx, c) in cats.iter().enumerate() {
        let header = if c.header.is_empty() {
            "categories".to_string()
        } else {
            c.header.clone()
        };
        if let Some(g) = groups.iter_mut().find(|g| g.0 == header) {
            g.1.push((idx, c));
        } else {
            groups.push((header, vec![(idx, c)]));
        }
    }

    // Layout constants — 2 columns of bordered chip buttons, 3 rows tall each.
    let chip_h: u16 = 3;
    let row_gap: u16 = 0;
    let group_header_h: u16 = 2; // separator line + 1 spacer
    let col_count: u16 = 2;
    let col_gap: u16 = 2;
    let total_gap_w = col_gap * (col_count - 1);

    // Pre-compute total content height for scroll math.
    let mut total_h: u16 = 0;
    for (_, members) in &groups {
        total_h += group_header_h;
        let rows = ((members.len() as u16) + col_count - 1) / col_count;
        total_h += rows * (chip_h + row_gap);
        total_h += 1; // group bottom spacer
    }

    // Clamp scroll.
    let max_scroll = total_h.saturating_sub(area.height);
    let scroll = app.filters_scroll.min(max_scroll);
    app.filters_scroll = scroll;

    // Reserve a 1-col scrollbar gutter when there's overflow.
    let needs_scroll = total_h > area.height;
    let (content_rect, sb_rect) = if needs_scroll {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        (chunks[0], Some(chunks[1]))
    } else {
        (area, None)
    };
    let content_col_w = content_rect.width.saturating_sub(total_gap_w) / col_count;

    // Virtual cursor in "content space" (0 = top of all content).
    let visible_top = scroll;
    let visible_bottom = scroll + content_rect.height;
    let mut vy: u16 = 0;

    for (header, members) in &groups {
        // Section header bar
        if vy + group_header_h > visible_top && vy < visible_bottom {
            let actual_y = content_rect.y + vy.saturating_sub(visible_top);
            if actual_y < content_rect.y + content_rect.height {
                let header_label = pretty_category(header);
                let header_text = format!("  {header_label}  ");
                let dash_w = (content_rect.width as usize)
                    .saturating_sub(header_text.chars().count());
                let line = Line::from(vec![
                    Span::styled(
                        header_text,
                        Style::default()
                            .fg(theme::ACCENT)
                            .bg(theme::BG)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "─".repeat(dash_w),
                        Style::default().fg(theme::BORDER).bg(theme::BG),
                    ),
                ]);
                f.render_widget(
                    Paragraph::new(line).style(theme::base()),
                    Rect::new(content_rect.x, actual_y, content_rect.width, 1),
                );
            }
        }
        vy += group_header_h;

        // Chip grid
        for (i, (idx, cat)) in members.iter().enumerate() {
            let col = (i as u16) % col_count;
            let row = (i as u16) / col_count;
            let chip_vy = vy + row * (chip_h + row_gap);
            // Only render chips whose vertical span overlaps the visible window.
            if chip_vy + chip_h <= visible_top || chip_vy >= visible_bottom {
                continue;
            }
            let actual_y = content_rect.y + chip_vy.saturating_sub(visible_top);
            // If the chip would render past the bottom, skip (avoid partial-button glitches).
            if actual_y + chip_h > content_rect.y + content_rect.height {
                continue;
            }
            let chip_x = content_rect.x + col * (content_col_w + col_gap);
            let chip_rect = Rect::new(chip_x, actual_y, content_col_w, chip_h);
            let selected = app.selected_categories.iter().any(|s| s == &cat.name);
            draw_chip_button(f, app, chip_rect, &pretty_category(&cat.name), selected, Hit::CategoryChip(*idx));
        }
        let rows = ((members.len() as u16) + col_count - 1) / col_count;
        vy += rows * (chip_h + row_gap);
        vy += 1;
    }

    // Scrollbar
    if let Some(sb_rect) = sb_rect {
        let mut sb_state = ScrollbarState::new(max_scroll as usize)
            .position(scroll as usize)
            .viewport_content_length(content_rect.height as usize);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme::BORDER).bg(theme::BG))
            .thumb_style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(sb, sb_rect, &mut sb_state);
    }
}

fn draw_chip_button(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    label: &str,
    selected: bool,
    hit: Hit,
) {
    let hovered = app.hover == Some(hit);
    let (border_fg, label_fg, modifier) = if selected {
        (theme::GOLD, theme::GOLD, Modifier::BOLD)
    } else if hovered {
        (theme::ACCENT_HI, theme::ACCENT_HI, Modifier::BOLD)
    } else {
        (theme::BORDER, theme::FG, Modifier::empty())
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_fg).bg(theme::BG))
        .style(theme::base());
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let prefix = if selected { "✓  " } else { "" };
    let text = format!("{prefix}{label}");
    let mid = inner.height / 2;
    let styled = Span::styled(
        text,
        Style::default()
            .fg(label_fg)
            .bg(theme::BG)
            .add_modifier(modifier),
    );
    let lines: Vec<Line> = (0..inner.height)
        .map(|i| {
            if i == mid {
                Line::from(vec![styled.clone()])
            } else {
                Line::from("")
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(theme::base()),
        inner,
    );
    app.click_regions.push((rect, hit));
}

fn draw_info_popup(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(popup) = app.info_popup.clone() else {
        return;
    };
    let title_text = format!(" {} ", popup.title);
    let message = popup.body;
    let w = 60u16.min(area.width.saturating_sub(4));
    // text width inside borders + 2-col horizontal margin
    let text_w = w.saturating_sub(6) as usize;
    let wrapped_lines: u16 = message
        .lines()
        .map(|l| {
            let chars = l.chars().count();
            if chars == 0 {
                1
            } else {
                ((chars + text_w - 1) / text_w).max(1) as u16
            }
        })
        .sum();
    // borders(2) + vertical margin(2) + text + 1 gap + button(3) = wrapped_lines + 8
    let h = (wrapped_lines + 8).min(area.height.saturating_sub(4)).max(8);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);

    // Paint over whatever was underneath before drawing the modal.
    f.render_widget(Fill { style: theme::base() }, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .style(theme::base())
        .title(Span::styled(title_text, theme::accent_bold()));
    let inner = block.inner(rect).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, rect);

    let text_h = inner.height.saturating_sub(BUTTON_H + 1);
    let text_rect = Rect::new(inner.x, inner.y, inner.width, text_h);
    f.render_widget(
        Paragraph::new(message)
            .style(theme::base())
            .wrap(Wrap { trim: true }),
        text_rect,
    );
    let _ = app;

    let btn_y = inner.y + inner.height.saturating_sub(BUTTON_H);
    let btn_row = Rect::new(inner.x, btn_y, inner.width, BUTTON_H);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(10), Constraint::Min(0)])
        .split(btn_row);
    draw_button(f, app, cols[1], "OK", Hit::DismissInfoPopup, true);
}

fn update_modal_visible(app: &App) -> bool {
    if app.update_modal_dismissed {
        return false;
    }
    matches!(
        app.update_status,
        UpdateStatus::Outdated(_)
            | UpdateStatus::Downloading { .. }
            | UpdateStatus::Ready { .. }
    )
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
    if app.viewing_news.is_some() {
        draw_article(f, app, area);
        return;
    }
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
    app.play_inner = inner;

    let installing = app.install.is_some();
    let progress_h = if installing { 2 } else { 0 };
    let progress_gap = if installing { 1 } else { 0 };
    // Default top: selected(1) + playing(1) + gap(1) + button(3) + gap(1) + progress + progress_gap
    let default_top = 7u16 + progress_h + progress_gap;
    let max_top = inner.height.saturating_sub(4);
    let top_h = app.news_split_top.unwrap_or(default_top).clamp(7, max_top.max(7));

    let outer_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_h),
            Constraint::Length(1), // news header (draggable)
            Constraint::Min(0),    // news list
        ])
        .split(inner);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),            // 0 Selected
            Constraint::Length(1),            // 1 Playing as
            Constraint::Length(1),            // 2 gap
            Constraint::Length(BUTTON_H),     // 3 button
            Constraint::Length(progress_gap), // 4 (gap only if installing)
            Constraint::Length(progress_h),   // 5 progress (0 if not installing)
            Constraint::Min(0),               // padding
        ])
        .split(outer_rows[0]);

    let modded = app.filter == VersionFilter::Modded;
    let sel = match app.selected_manifest_entry() {
        Some(v) if modded => format!("{}  ·  Fabric mod loader", v.id),
        Some(v) => format!("{} ({})", v.id, v.kind.label()),
        None => "(no version selected)".to_string(),
    };
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
        .constraints([
            Constraint::Length(22),
            Constraint::Length(2),
            Constraint::Length(20),
            Constraint::Min(0),
        ])
        .split(rows[3]);
    let installed = if modded {
        app.selected_modded_installed()
    } else {
        app.selected_is_installed()
    };
    if let Some(state) = &app.install {
        let label = match state.kind {
            InstallKind::Install => "Installing...",
            InstallKind::Verify => "Verifying...",
        };
        draw_disabled_button(f, btn_cols[0], label);
    } else if app.launch_state == LaunchState::Running {
        draw_disabled_button(f, btn_cols[0], "Running");
    } else if installed {
        draw_button(f, app, btn_cols[0], "▶  Launch", Hit::LaunchButton, true);
    } else {
        draw_button(f, app, btn_cols[0], "⬇  Install", Hit::InstallButton, true);
    }
    if modded {
        if installed {
            draw_button(f, app, btn_cols[2], "📦 Browse Mods", Hit::BrowseModsButton, false);
        } else {
            draw_dim_clickable_button(f, app, btn_cols[2], "📦 Browse Mods", Hit::BrowseModsButton);
        }
    }

    if installing {
        draw_progress(f, app, rows[5]);
    }
    draw_news_header(f, app, outer_rows[1]);
    draw_news_list(f, app, outer_rows[2]);
}

fn draw_news_header(f: &mut Frame, app: &mut App, area: Rect) {
    let grip_hovered = app.hover == Some(Hit::NewsSplitter) || app.dragging_split;
    let grip_char = if grip_hovered { "⇕ " } else { "↕ " };
    let title_text = format!("{grip_char}Release notes ");
    let link_text = "See all on minecraft.net ↗";
    let right_pad = "  ";
    let title_w = title_text.chars().count();
    let link_w = link_text.chars().count();
    let pad_w = right_pad.chars().count();
    let area_w = area.width as usize;
    let dash_w = area_w.saturating_sub(title_w + link_w + pad_w);

    let title_style = if grip_hovered {
        Style::default()
            .fg(theme::ACCENT_HI)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::accent_bold()
    };

    let link_hovered = app.hover == Some(Hit::OpenAllArticles);
    let link_style = if link_hovered {
        Style::default()
            .fg(theme::ACCENT_HI)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else {
        Style::default().fg(theme::ACCENT).bg(theme::BG)
    };

    let dash_style = if grip_hovered {
        Style::default().fg(theme::ACCENT).bg(theme::BG)
    } else {
        Style::default().fg(theme::BORDER).bg(theme::BG)
    };

    let line = Line::from(vec![
        Span::styled(title_text, title_style),
        Span::styled("─".repeat(dash_w), dash_style),
        Span::styled(link_text, link_style),
        Span::styled(right_pad, theme::base()),
    ]);
    f.render_widget(Paragraph::new(line).style(theme::base()), area);

    let grip_rect = Rect::new(area.x, area.y, (title_w + dash_w) as u16, 1);
    app.click_regions.push((grip_rect, Hit::NewsSplitter));

    let link_rect = Rect::new(
        area.x + (title_w + dash_w) as u16,
        area.y,
        link_w as u16,
        1,
    );
    app.click_regions.push((link_rect, Hit::OpenAllArticles));
}

fn draw_news_list(f: &mut Frame, app: &mut App, area: Rect) {
    if app.news.is_empty() {
        f.render_widget(
            Paragraph::new("Loading news from minecraft.net...").style(theme::dim()),
            area,
        );
        return;
    }
    let total = app.news.len();
    let h = area.height as usize;
    if h == 0 {
        return;
    }
    let has_scroll = total > h;

    // Reserve a 1-col scrollbar gutter if we need one.
    let chunks = if has_scroll {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0)])
            .split(area)
    };
    let list_area = chunks[0];

    if total > h && app.news_offset + h > total {
        app.news_offset = total - h;
    }
    if total <= h {
        app.news_offset = 0;
    }
    let start = app.news_offset;
    let end = (start + h).min(total);

    for (row_i, entry) in app.news[start..end].iter().enumerate() {
        let global_i = start + row_i;
        let y = list_area.y + row_i as u16;
        let rect = Rect::new(list_area.x, y, list_area.width, 1);
        let hovered = app.hover == Some(Hit::NewsItem(global_i));
        let title_style = if hovered {
            Style::default()
                .fg(theme::ACCENT_HI)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme::FG).bg(theme::BG)
        };
        let kind = if entry.kind.is_empty() {
            String::new()
        } else {
            format!("  · {}", entry.kind)
        };
        let line = Line::from(vec![
            Span::styled("▸ ", Style::default().fg(theme::ACCENT)),
            Span::styled(format!("{}  ", entry.date_short()), theme::dim()),
            Span::styled(entry.title.clone(), title_style),
            Span::styled(kind, theme::dim()),
        ]);
        f.render_widget(Paragraph::new(line).style(theme::base()), rect);
        app.click_regions.push((rect, Hit::NewsItem(global_i)));
    }

    if has_scroll {
        let scroll_range = total - h;
        let mut sb_state = ScrollbarState::new(scroll_range)
            .position(app.news_offset)
            .viewport_content_length(h);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme::BORDER).bg(theme::BG))
            .thumb_style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(sb, chunks[1], &mut sb_state);
        app.click_regions.push((chunks[1], Hit::NewsScrollbar));
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
        .constraints([Constraint::Length(64), Constraint::Min(0)])
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
            (
                "Modded",
                Hit::FilterModded,
                app.filter == VersionFilter::Modded,
            ),
        ],
    );

    if app.filter == VersionFilter::Modded {
        let hint = match app.latest_stable_fabric_loader() {
            Some(v) => format!("  Loader: Fabric {v}  ·  install via Play tab"),
            None => "  Loader: Fabric (loading...)".to_string(),
        };
        let hint_rect = filter_cols[1];
        let mid = hint_rect.height / 2;
        let lines: Vec<Line> = (0..hint_rect.height)
            .map(|i| {
                if i == mid {
                    Line::from(Span::styled(hint.clone(), theme::dim()))
                } else {
                    Line::from("")
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines).style(theme::base()), hint_rect);
    }

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

    let modded = app.filter == VersionFilter::Modded;
    for (i, (id, kind_label, date)) in snapshot[start..end].iter().enumerate() {
        let global_idx = start + i;
        let y = content_rect.y + i as u16;
        let rect = Rect::new(content_rect.x, y, content_rect.width, 1);
        let selected = app.selected_version.as_deref() == Some(id.as_str());
        let hovered = app.hover == Some(Hit::VersionRow(global_idx));
        let installed = if modded {
            app.modded_id_for(id)
                .map(|mid| app.is_installed(&mid))
                .unwrap_or(false)
        } else {
            app.is_installed(id)
        };
        let row_style = if selected {
            theme::list_selected()
        } else if hovered {
            theme::button_idle()
        } else {
            theme::base()
        };
        let marker = if selected { "▶" } else { " " };
        let prefix = format!(" {marker} ");
        let check = if installed { "✓ " } else { "  " };
        let body = format!("{:<18} {:<10} {date} ", id, kind_label);
        let check_style = if installed {
            let mut s = row_style;
            s = s.fg(theme::ACCENT_HI).add_modifier(Modifier::BOLD);
            s
        } else {
            row_style
        };
        let line = Line::from(vec![
            Span::styled(prefix, row_style),
            Span::styled(check.to_string(), check_style),
            Span::styled(body, row_style),
        ]);
        f.render_widget(Paragraph::new(line).style(row_style), rect);
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
        app.click_regions.push((sb_rect, Hit::VersionsScrollbar));
    }
}

fn draw_accounts(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Profile ", theme::accent_bold()));
    let full_inner = block.inner(area).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, area);

    // Split off a preview column on the right when there's room.
    let show_preview = full_inner.width >= 60;
    let (inner, preview_area) = if show_preview {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(2), Constraint::Length(20)])
            .split(full_inner);
        let preview_height = cols[2].height.min(SKIN_PREVIEW_BOX_H);
        (
            cols[0],
            Some(Rect::new(
                cols[2].x,
                cols[2].y,
                cols[2].width,
                preview_height,
            )),
        )
    } else {
        (full_inner, None)
    };

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
                    format!("● {}", a.username),
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
            .constraints([Constraint::Length(18), Constraint::Min(0)])
            .split(btn_row_rect);
        if app.account.is_some() {
            draw_button(f, app, cols[0], "Sign out", Hit::LogoutButton, false);
        } else {
            draw_button(f, app, cols[0], "Sign in (MS)", Hit::LoginButton, true);
        }
        y += BUTTON_H + 1;

        if app.account.is_some() {
            draw_skin_section(f, app, inner, y, true);
        }
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
        y += BUTTON_H + 1;
        draw_skin_section(f, app, inner, y, false);
    }

    if let Some(prev_area) = preview_area {
        draw_skin_preview_box(f, app, prev_area);
    }
}

fn draw_skin_section(f: &mut Frame, app: &mut App, inner: Rect, mut y: u16, online: bool) {
    let bottom = inner.y + inner.height;
    if y + 1 >= bottom {
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("Skin", theme::accent_bold())))
            .style(theme::base()),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 1;
    if y + 1 >= bottom {
        return;
    }
    let sep_w = inner.width as usize;
    f.render_widget(
        Paragraph::new("─".repeat(sep_w))
            .style(Style::default().fg(theme::BORDER).bg(theme::BG)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 2;
    if y + BUTTON_H >= bottom {
        return;
    }

    if online {
        let label_rect = Rect::new(inner.x, y, 7, BUTTON_H);
        draw_vcentered_label(f, "Model:", label_rect, theme::dim());
        let seg_rect = Rect::new(inner.x + 8, y, 34, BUTTON_H);
        draw_segmented(
            f,
            app,
            seg_rect,
            &[
                ("Classic", Hit::SkinModelClassic, app.skin_model == SkinModel::Classic),
                ("Slim", Hit::SkinModelSlim, app.skin_model == SkinModel::Slim),
            ],
        );
        y += BUTTON_H + 1;
        if y + BUTTON_H >= bottom {
            return;
        }
    }

    let label_rect = Rect::new(inner.x, y, 11, BUTTON_H);
    draw_vcentered_label(f, "URL / user:", label_rect, theme::dim());
    let url_w = inner.width.saturating_sub(12).min(54);
    let url_rect = Rect::new(inner.x + 12, y, url_w, BUTTON_H);
    draw_skin_url(f, app, url_rect);
    y += BUTTON_H + 1;
    if y + BUTTON_H >= bottom {
        return;
    }

    let btn_row = Rect::new(inner.x, y, inner.width, BUTTON_H);
    let apply_label = if online { "Apply skin" } else { "Save skin" };
    let constraints: Vec<Constraint> = if online {
        vec![
            Constraint::Length(13),
            Constraint::Length(2),
            Constraint::Length(13),
            Constraint::Length(2),
            Constraint::Length(20),
            Constraint::Min(0),
        ]
    } else {
        vec![
            Constraint::Length(13),
            Constraint::Length(2),
            Constraint::Length(13),
            Constraint::Min(0),
        ]
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(btn_row);

    if app.skin_pending_loading {
        draw_disabled_button(f, cols[0], "Loading...");
    } else {
        draw_button(f, app, cols[0], "Preview", Hit::PreviewSkinButton, false);
    }
    if app.skin_busy {
        draw_disabled_button(f, cols[2], "Working...");
    } else {
        draw_button(f, app, cols[2], apply_label, Hit::ApplySkinButton, true);
    }
    if online {
        draw_button(f, app, cols[4], "Reset to default", Hit::ResetSkinButton, false);
    }
    y += BUTTON_H + 1;

    if !online && y + 1 < bottom {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Saved URL is used by skin-loader mods (CustomSkinLoader, OfflineSkins) — vanilla Minecraft can't render custom skins offline by itself.",
                theme::dim(),
            )))
            .style(theme::base())
            .wrap(Wrap { trim: true }),
            Rect::new(inner.x, y, inner.width, bottom.saturating_sub(y).min(3)),
        );
    }
}

fn draw_skin_url(f: &mut Frame, app: &mut App, rect: Rect) {
    let focused = app.focus == Focus::SkinUrl;
    let hovered = app.hover == Some(Hit::SkinUrlField);
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
    let inner_rect = block.inner(rect);
    f.render_widget(block, rect);

    let content = if focused {
        format!("{}▎", app.skin_url_input)
    } else if app.skin_url_input.is_empty() {
        "(paste a skin URL or a Minecraft username like Notch)".to_string()
    } else {
        app.skin_url_input.clone()
    };
    let style = if !focused && app.skin_url_input.is_empty() {
        theme::dim()
    } else {
        theme::base()
    };
    draw_vcentered_label(f, &format!(" {content}"), inner_rect, style);

    app.click_regions.push((rect, Hit::SkinUrlField));
}

fn draw_skin_preview_box(f: &mut Frame, app: &mut App, area: Rect) {
    let pending = app.skin_pending_preview.is_some();
    let (title, border_fg) = if pending {
        (" Pending preview ", theme::GOLD)
    } else {
        (" Skin preview ", theme::BORDER)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_fg))
        .style(theme::base())
        .title(Span::styled(
            title,
            Style::default()
                .fg(if pending { theme::GOLD } else { theme::ACCENT })
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let preview_to_show = app
        .skin_pending_preview
        .as_ref()
        .or(app.skin_preview.as_ref());

    match preview_to_show {
        Some(preview) => {
            let cols = preview.cols();
            let rows = preview.rows();
            let x = inner.x + inner.width.saturating_sub(cols) / 2;
            let y = inner.y + inner.height.saturating_sub(rows + 2) / 2;
            let preview_rect = Rect::new(x, y, cols.min(inner.width), rows.min(inner.height));
            f.render_widget(
                SkinPreviewWidget {
                    preview,
                    view: app.skin_view,
                },
                preview_rect,
            );

            // Rotation arrows sit on the preview box border.
            let mid_y = preview_rect.y + preview_rect.height / 2;
            if area.width > 2 && area.height > 2 {
                let left_rect = Rect::new(area.x, mid_y, 1, 1);
                let hovered = app.hover == Some(Hit::RotateSkinLeft);
                let style = if hovered {
                    Style::default()
                        .fg(theme::ACCENT_HI)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::ACCENT).bg(theme::BG)
                };
                f.render_widget(Paragraph::new("◀").style(style), left_rect);
                app.click_regions.push((left_rect, Hit::RotateSkinLeft));

                let right_x = area.x + area.width - 1;
                let right_rect = Rect::new(right_x, mid_y, 1, 1);
                let hovered = app.hover == Some(Hit::RotateSkinRight);
                let style = if hovered {
                    Style::default()
                        .fg(theme::ACCENT_HI)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::ACCENT).bg(theme::BG)
                };
                f.render_widget(Paragraph::new("▶").style(style), right_rect);
                app.click_regions.push((right_rect, Hit::RotateSkinRight));
            }

            let label_y = preview_rect.y + rows;
            if label_y < inner.y + inner.height {
                let label_rect = Rect::new(inner.x, label_y, inner.width, 1);
                f.render_widget(
                    Paragraph::new(app.skin_view.label())
                        .style(theme::dim())
                        .alignment(Alignment::Center),
                    label_rect,
                );
            }

            if pending && inner.height > rows + 2 {
                let hint_y = preview_rect.y + rows + 1;
                let hint_rect = Rect::new(inner.x, hint_y, inner.width, 1);
                let hovered = app.hover == Some(Hit::ClearPreviewButton);
                let style = if hovered {
                    Style::default()
                        .fg(theme::ACCENT_HI)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(theme::FG_DIM).bg(theme::BG)
                };
                f.render_widget(
                    Paragraph::new("× clear preview")
                        .style(style)
                        .alignment(Alignment::Center),
                    hint_rect,
                );
                app.click_regions.push((hint_rect, Hit::ClearPreviewButton));
            }
        }
        None => {
            let msg = if app.account.is_some() {
                "Loading skin..."
            } else {
                "Sign in to see your skin"
            };
            f.render_widget(
                Paragraph::new(msg)
                    .style(theme::dim())
                    .alignment(Alignment::Center),
                inner,
            );
        }
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
        app.click_regions.push((sb_area, Hit::LogsScrollbar));
    }
}

fn draw_disabled_button(f: &mut Frame, rect: Rect, label: &str) {
    draw_disabled_button_inner(f, rect, label, None);
}

fn draw_dim_clickable_button(f: &mut Frame, app: &mut App, rect: Rect, label: &str, hit: Hit) {
    draw_disabled_button_inner(f, rect, label, Some(hit));
    app.click_regions.push((rect, hit));
}

fn draw_disabled_button_inner(f: &mut Frame, rect: Rect, label: &str, _click: Option<Hit>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER).bg(theme::BG));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mid = inner.height / 2;
    let styled = Span::styled(
        label,
        Style::default().fg(theme::FG_DIM).bg(theme::BG),
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

    let line = Line::from(vec![
        dot.clone(),
        Span::styled(acct, theme::dim()),
        sp.clone(),
        dot.clone(),
        Span::styled(java, theme::dim()),
        sp,
        dot,
        Span::styled(app.status_message.clone(), theme::dim()),
    ]);
    f.render_widget(Paragraph::new(line).style(theme::base()), area);
}

fn draw_article(f: &mut Frame, app: &mut App, area: Rect) {
    let (title, date, kind, source_url) = match &app.article {
        Some(a) => (
            a.title.clone(),
            a.date.get(..10).unwrap_or(&a.date).to_string(),
            a.kind.clone(),
            a.source_url.clone(),
        ),
        None => (
            app.viewing_news
                .and_then(|i| app.news.get(i).map(|e| e.title.clone()))
                .unwrap_or_default(),
            String::new(),
            String::new(),
            "https://www.minecraft.net/en-us/articles".into(),
        ),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(
            format!(" 🌐  {source_url} "),
            Style::default()
                .fg(theme::ACCENT)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // close button row
            Constraint::Length(1), // gap
            Constraint::Length(2), // title (1 line) + spacer
            Constraint::Length(1), // metadata
            Constraint::Length(1), // separator
            Constraint::Min(0),    // body
        ])
        .split(inner);

    // Close button on the right
    let close_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(8)])
        .split(rows[0]);
    let close_rect = close_cols[1];
    let hovered = app.hover == Some(Hit::CloseArticle);
    let close_style = if hovered {
        Style::default()
            .fg(theme::ACCENT_HI)
            .bg(theme::BG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG_DIM).bg(theme::BG)
    };
    f.render_widget(
        Paragraph::new("× Close")
            .style(close_style)
            .alignment(Alignment::Right),
        close_rect,
    );
    app.click_regions.push((close_rect, Hit::CloseArticle));

    f.render_widget(
        Paragraph::new(Span::styled(
            title,
            Style::default()
                .fg(theme::ACCENT_HI)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme::base()),
        rows[2],
    );

    let meta = if date.is_empty() && kind.is_empty() {
        String::new()
    } else if kind.is_empty() {
        date
    } else {
        format!("{date}  ·  {kind}")
    };
    f.render_widget(
        Paragraph::new(Span::styled(meta, theme::dim())).style(theme::base()),
        rows[3],
    );

    let sep: String = "─".repeat(rows[4].width as usize);
    f.render_widget(
        Paragraph::new(sep).style(Style::default().fg(theme::BORDER).bg(theme::BG)),
        rows[4],
    );

    let body_area = rows[5];
    if app.article_loading {
        f.render_widget(
            Paragraph::new("Loading...").style(theme::dim()),
            body_area,
        );
        return;
    }
    let Some(article) = &app.article else {
        f.render_widget(
            Paragraph::new("(no content)").style(theme::dim()),
            body_area,
        );
        return;
    };

    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(body_area);
    let content_area = body_cols[0];
    let sb_area = body_cols[1];

    let lines = article_to_lines(article);
    let total_lines = lines.len();
    let visible = content_area.height as usize;
    let max_offset = total_lines.saturating_sub(visible) as u16;
    if app.article_offset > max_offset {
        app.article_offset = max_offset;
    }
    let para = Paragraph::new(lines)
        .style(theme::base())
        .wrap(Wrap { trim: false })
        .scroll((app.article_offset, 0));
    f.render_widget(para, content_area);

    if total_lines > visible {
        let mut sb_state = ScrollbarState::new(max_offset as usize)
            .position(app.article_offset as usize)
            .viewport_content_length(visible);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .style(Style::default().fg(theme::BORDER).bg(theme::BG))
            .thumb_style(Style::default().fg(theme::ACCENT).bg(theme::BG))
            .begin_symbol(None)
            .end_symbol(None);
        f.render_stateful_widget(sb, sb_area, &mut sb_state);
        app.click_regions.push((sb_area, Hit::ArticleScrollbar));
    }

    if !article.read_more_link.is_empty() {
        let bottom_y = inner.y + inner.height.saturating_sub(1);
        let link_rect = Rect::new(inner.x, bottom_y, inner.width, 1);
        let hovered = app.hover == Some(Hit::OpenArticleExternal);
        let style = if hovered {
            Style::default()
                .fg(theme::ACCENT_HI)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(theme::ACCENT)
                .bg(theme::BG)
                .add_modifier(Modifier::UNDERLINED)
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                "↗ Read full article on minecraft.net",
                style,
            ))
            .style(theme::base()),
            link_rect,
        );
        app.click_regions.push((link_rect, Hit::OpenArticleExternal));
    }
}

fn article_to_lines(article: &crate::news::Article) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for block in &article.blocks {
        match block {
            ArticleBlock::Heading(level, text) => {
                lines.push(Line::from(""));
                let style = if *level <= 2 {
                    Style::default()
                        .fg(theme::ACCENT_HI)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default()
                        .fg(theme::ACCENT)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD)
                };
                lines.push(Line::from(Span::styled(text.clone(), style)));
                lines.push(Line::from(""));
            }
            ArticleBlock::Paragraph(text) => {
                lines.push(Line::from(Span::styled(
                    text.clone(),
                    Style::default().fg(theme::FG).bg(theme::BG),
                )));
                lines.push(Line::from(""));
            }
            ArticleBlock::Bullet(text) => {
                lines.push(Line::from(vec![
                    Span::styled("  • ", Style::default().fg(theme::ACCENT).bg(theme::BG)),
                    Span::styled(text.clone(), Style::default().fg(theme::FG).bg(theme::BG)),
                ]));
            }
        }
    }
    lines
}

pub fn article_line_count(article: &crate::news::Article) -> usize {
    article_to_lines(article).len()
}

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Settings ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, area);

    // Two-column layout: Updates+Game on the left, Java+Maintenance on the right.
    // Keeps everything inside 120x38 even with all sections expanded.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Length(2), Constraint::Min(0)])
        .split(inner);
    let left = cols[0];
    let right = cols[2];

    // --- LEFT COLUMN: Updates + Game ---
    let mut y = left.y;
    let bottom_left = left.y + left.height;
    let row = |yy: u16, x: u16, w: u16| Rect::new(x, yy, w, 1);

    y = draw_section_header(f, "Updates", left, y);

    if y + 1 <= bottom_left {
        let current_line = format!("Current: v{}", env!("CARGO_PKG_VERSION"));
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("● ", Style::default().fg(theme::ACCENT)),
                Span::styled(current_line, theme::base()),
            ]))
            .style(theme::base()),
            row(y, left.x, left.width),
        );
        y += 2;
    }

    if y + BUTTON_H <= bottom_left {
        let btn_row = Rect::new(left.x, y, left.width, BUTTON_H);
        let bcols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(22),
                Constraint::Length(1),
                Constraint::Length(20),
                Constraint::Min(0),
            ])
            .split(btn_row);
        match app.update_status {
            UpdateStatus::Checking => draw_disabled_button(f, bcols[0], "Checking..."),
            UpdateStatus::Downloading { .. } => draw_disabled_button(f, bcols[0], "Downloading..."),
            UpdateStatus::Ready { .. } => {
                draw_button(f, app, bcols[0], "Install & restart", Hit::InstallUpdateNow, true);
            }
            _ => {
                draw_button(f, app, bcols[0], "Check for updates", Hit::CheckUpdatesButton, true);
            }
        }
        let show_releases_btn = matches!(
            app.update_status,
            UpdateStatus::Outdated(_) | UpdateStatus::UpToDate(_) | UpdateStatus::Failed(_)
        );
        if show_releases_btn {
            draw_button(f, app, bcols[2], "Releases page", Hit::OpenReleasesPage, false);
        }
        y += BUTTON_H + 1;
    }

    let (status_line, status_style) = match &app.update_status {
        UpdateStatus::Idle => (
            "Click \"Check for updates\" to query GitHub for the latest release.".to_string(),
            theme::dim(),
        ),
        UpdateStatus::Checking => (
            "Contacting GitHub...".to_string(),
            Style::default().fg(theme::GOLD).bg(theme::BG),
        ),
        UpdateStatus::UpToDate(v) => (
            format!("✓ You're on the latest version (v{v})."),
            Style::default().fg(theme::ACCENT_HI).bg(theme::BG),
        ),
        UpdateStatus::Outdated(info) => (
            format!(
                "⚠  Update available: v{} → v{}.  Open the releases page to download.",
                info.current, info.latest
            ),
            Style::default().fg(theme::GOLD).bg(theme::BG),
        ),
        UpdateStatus::Downloading { info, done, total } => {
            let pct = if *total == 0 {
                0.0
            } else {
                (*done as f64 / *total as f64) * 100.0
            };
            (
                format!(
                    "⬇  Downloading v{}... {:.0}%",
                    info.latest, pct
                ),
                Style::default().fg(theme::GOLD).bg(theme::BG),
            )
        }
        UpdateStatus::Ready { info, .. } => (
            format!(
                "✓ v{} downloaded — click \"Install & restart\" in the popup, or hit it here.",
                info.latest
            ),
            Style::default().fg(theme::ACCENT_HI).bg(theme::BG),
        ),
        UpdateStatus::Failed(e) => (
            format!("✗ Update check failed: {e}"),
            Style::default().fg(theme::RED).bg(theme::BG),
        ),
    };
    // Compact update status (one line, no wrap).
    if y + 1 <= bottom_left {
        let max_len = left.width as usize;
        let mut s = status_line;
        if s.chars().count() > max_len {
            s = s.chars().take(max_len.saturating_sub(1)).collect::<String>() + "…";
        }
        f.render_widget(
            Paragraph::new(Span::styled(s, status_style)).style(theme::base()),
            row(y, left.x, left.width),
        );
        y += 2;
    }

    // --- Game section: RAM + Open data folder ---
    if y + 2 <= bottom_left {
        y = draw_section_header(f, "Game", left, y);
    }
    if y + 1 <= bottom_left {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Max RAM: ", theme::dim()),
                Span::styled(
                    format!("{} MB", app.max_ram_mb),
                    Style::default()
                        .fg(theme::ACCENT_HI)
                        .bg(theme::BG)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("   (512 – 32768)", theme::dim()),
            ]))
            .style(theme::base()),
            row(y, left.x, left.width),
        );
        y += 1;
    }
    if y + BUTTON_H <= bottom_left {
        let ram_btn_row = Rect::new(left.x, y, left.width, BUTTON_H);
        let ram_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(12),
                Constraint::Length(1),
                Constraint::Length(12),
                Constraint::Length(2),
                Constraint::Length(18),
                Constraint::Min(0),
            ])
            .split(ram_btn_row);
        draw_button(f, app, ram_cols[0], "−  512 MB", Hit::RamDecrease, false);
        draw_button(f, app, ram_cols[2], "+  512 MB", Hit::RamIncrease, false);
        draw_button(f, app, ram_cols[4], "Open data folder", Hit::OpenDataFolder, false);
    }

    // --- RIGHT COLUMN: Java + Maintenance ---
    let mut y = right.y;
    let bottom_right = right.y + right.height;

    if y + 2 <= bottom_right {
        y = draw_section_header(f, "Java", right, y);
    }
    if y + BUTTON_H <= bottom_right {
        let jp_row = Rect::new(right.x, y, right.width, BUTTON_H);
        let jp_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Length(9)])
            .split(jp_row);
        let jp_label_owned: String = if app.java_path_input.is_empty() {
            "(auto-detect)".to_string()
        } else {
            app.java_path_input.clone()
        };
        let jp_focused = app.focus == Focus::JavaPath;
        draw_text_field(
            f,
            app,
            jp_cols[0],
            Hit::JavaPathField,
            Focus::JavaPath,
            " Java path (all versions) ",
            &jp_label_owned,
            jp_focused,
        );
        if !app.java_path_input.is_empty() {
            draw_button(f, app, jp_cols[2], "Clear", Hit::ClearJavaPath, false);
        }
        y += BUTTON_H + 1;
    }
    if y + BUTTON_H <= bottom_right {
        let jpv_row = Rect::new(right.x, y, right.width, BUTTON_H);
        let jpv_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(1), Constraint::Length(9)])
            .split(jpv_row);
        let pv_label_owned: String = if app.focus == Focus::JavaPathForVersion {
            app.java_path_for_version_input.clone()
        } else {
            let id = app
                .selected_modded_id()
                .or_else(|| app.selected_version.clone());
            match id.as_deref().and_then(|i| app.java_path_per_version.get(i)) {
                Some(v) if !v.is_empty() => v.clone(),
                _ => "(none)".to_string(),
            }
        };
        let pv_focused = app.focus == Focus::JavaPathForVersion;
        let pv_title = match app
            .selected_modded_id()
            .or_else(|| app.selected_version.clone())
        {
            Some(id) if id.chars().count() < 24 => format!(" Java path · {id} "),
            _ => " Java path (this version) ".to_string(),
        };
        draw_text_field(
            f,
            app,
            jpv_cols[0],
            Hit::JavaPathForVersionField,
            Focus::JavaPathForVersion,
            &pv_title,
            &pv_label_owned,
            pv_focused,
        );
        let id_for_clear = app
            .selected_modded_id()
            .or_else(|| app.selected_version.clone());
        let has_pv_override = id_for_clear
            .as_deref()
            .and_then(|i| app.java_path_per_version.get(i))
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        if has_pv_override {
            draw_button(f, app, jpv_cols[2], "Clear", Hit::ClearJavaPathForVersion, false);
        }
        y += BUTTON_H + 1;
    }

    if y + 2 <= bottom_right {
        y = draw_section_header(f, "Maintenance", right, y);
    }
    if y + BUTTON_H <= bottom_right {
        let verify_row = Rect::new(right.x, y, right.width, BUTTON_H);
        let verify_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(22), Constraint::Min(0)])
            .split(verify_row);
        if app.integrity_in_progress {
            draw_disabled_button(f, verify_cols[0], "Verifying...");
        } else {
            draw_button(
                f,
                app,
                verify_cols[0],
                "Verify integrity",
                Hit::VerifyIntegrityButton,
                false,
            );
        }
        // Compact one-line hint to the right of the button.
        let mid = verify_cols[1].y + verify_cols[1].height / 2;
        f.render_widget(
            Paragraph::new(Span::styled(
                "Re-hashes every file & repairs corruption.",
                theme::dim(),
            ))
            .style(theme::base()),
            Rect::new(verify_cols[1].x + 1, mid, verify_cols[1].width.saturating_sub(1), 1),
        );
    }
}

fn draw_section_header(f: &mut Frame, title: &str, inner: Rect, mut y: u16) -> u16 {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(title.to_string(), theme::accent_bold())))
            .style(theme::base()),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y += 1;
    f.render_widget(
        Paragraph::new("─".repeat(inner.width as usize))
            .style(Style::default().fg(theme::BORDER).bg(theme::BG)),
        Rect::new(inner.x, y, inner.width, 1),
    );
    y + 1
}

fn draw_text_field(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    hit: Hit,
    _focus_kind: Focus,
    title: &str,
    content: &str,
    focused: bool,
) {
    let hovered = app.hover == Some(hit);
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
        .style(theme::base())
        .title(Span::styled(title.to_string(), theme::dim()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let display = if focused {
        format!(" {content}▎")
    } else {
        format!(" {content}")
    };
    let style = if !focused && content.starts_with('(') {
        theme::dim()
    } else {
        theme::base()
    };
    draw_vcentered_label(f, &display, inner, style);
    app.click_regions.push((rect, hit));
}

fn draw_mod_browser(f: &mut Frame, app: &mut App, area: Rect) {
    wipe(f, area);
    let title = format!(
        " Content browser · {} · {} ",
        ModLoader::Fabric.label(),
        app.selected_version.clone().unwrap_or_else(|| "?".into())
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::ACCENT))
        .style(theme::base())
        .title(Span::styled(title, theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, area);

    // Top row: tab strip + close button
    let top_row = Rect::new(inner.x, inner.y, inner.width, 1);
    let tabs_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(10),
            Constraint::Length(1),
            Constraint::Length(10),
            Constraint::Length(1),
            Constraint::Length(8),
        ])
        .split(top_row);
    draw_browser_tabs(f, app, tabs_cols[0]);

    let export_rect = tabs_cols[1];
    let hovered_e = app.hover == Some(Hit::ExportProfileButton);
    let style_e = if hovered_e {
        Style::default().fg(theme::ACCENT_HI).bg(theme::BG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::ACCENT).bg(theme::BG)
    };
    f.render_widget(
        Paragraph::new("⤓ Export").style(style_e).alignment(Alignment::Right),
        export_rect,
    );
    app.click_regions.push((export_rect, Hit::ExportProfileButton));

    let import_rect = tabs_cols[3];
    let hovered_i = app.hover == Some(Hit::ImportProfileButton);
    let style_i = if hovered_i {
        Style::default().fg(theme::ACCENT_HI).bg(theme::BG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::ACCENT).bg(theme::BG)
    };
    f.render_widget(
        Paragraph::new("⤒ Import").style(style_i).alignment(Alignment::Right),
        import_rect,
    );
    app.click_regions.push((import_rect, Hit::ImportProfileButton));

    let close_rect = tabs_cols[5];
    let hovered = app.hover == Some(Hit::CloseModBrowser);
    let cs = if hovered {
        Style::default().fg(theme::ACCENT_HI).bg(theme::BG).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::FG_DIM).bg(theme::BG)
    };
    f.render_widget(
        Paragraph::new("× Close").style(cs).alignment(Alignment::Right),
        close_rect,
    );
    app.click_regions.push((close_rect, Hit::CloseModBrowser));

    // Separator under tabs
    let sep_y = inner.y + 1;
    f.render_widget(
        Paragraph::new("─".repeat(inner.width as usize))
            .style(Style::default().fg(theme::BORDER).bg(theme::BG)),
        Rect::new(inner.x, sep_y, inner.width, 1),
    );

    // Body
    let body_y = inner.y + 2;
    let body = Rect::new(
        inner.x,
        body_y,
        inner.width,
        inner.height.saturating_sub(body_y - inner.y),
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Length(2), Constraint::Min(0)])
        .split(body);
    let left = cols[0];
    let right = cols[2];

    draw_mod_search_pane(f, app, left);
    draw_installed_mods_pane(f, app, right);
}

fn draw_browser_tabs(f: &mut Frame, app: &mut App, area: Rect) {
    let mut x = area.x;
    for kind in ContentKind::ALL {
        let label = format!(" {} ", kind.label());
        let w = label.chars().count() as u16;
        if x + w > area.x + area.width {
            break;
        }
        let rect = Rect::new(x, area.y, w, 1);
        let active = app.browser_kind == kind;
        let hit = match kind {
            ContentKind::Mods => Hit::BrowserTabMods,
            ContentKind::Shaders => Hit::BrowserTabShaders,
            ContentKind::ResourcePacks => Hit::BrowserTabResourcePacks,
        };
        let disabled = matches!(kind, ContentKind::Shaders) && !app.shaders_available();
        let hovered = app.hover == Some(hit);
        let style = if active {
            Style::default()
                .fg(theme::ACCENT_HI)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else if disabled {
            Style::default().fg(theme::FG_DIM).bg(theme::BG)
        } else if hovered {
            Style::default().fg(theme::ACCENT).bg(theme::BG).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::FG).bg(theme::BG)
        };
        let suffix = if disabled { " 🔒" } else { "" };
        let display = format!("{label}{suffix}");
        f.render_widget(Paragraph::new(display).style(style), rect);
        // Even disabled tabs are clickable — clicking shows an info popup.
        app.click_regions.push((rect, hit));
        x += w + 2;
    }
}

fn draw_mod_search_pane(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(BUTTON_H),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    // Split the top row: search field on the left, Filters button + Installed toggle on the right.
    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(15),
            Constraint::Length(1),
            Constraint::Length(18),
        ])
        .split(rows[0]);
    draw_mod_search_field(f, app, top_cols[0]);
    let installed_label = if app.installed_filter_only {
        "✓ Installed"
    } else {
        "  All"
    };
    draw_button(
        f,
        app,
        top_cols[1],
        installed_label,
        Hit::InstalledFilterToggle,
        app.installed_filter_only,
    );
    let count = app.selected_categories.len();
    let filter_label = if count == 0 {
        "▼ Filters".to_string()
    } else {
        format!("▼ Filters ({count})")
    };
    draw_button(
        f,
        app,
        top_cols[3],
        &filter_label,
        Hit::OpenFiltersButton,
        count > 0,
    );

    let list_area = rows[2];
    wipe(f, list_area);

    if app.mod_search_loading && app.mod_search_results.is_empty() {
        f.render_widget(
            Paragraph::new("Loading from Modrinth...").style(theme::dim()),
            list_area,
        );
        return;
    }
    if let Some(e) = &app.mod_search_error {
        f.render_widget(
            Paragraph::new(format!("Error: {e}"))
                .style(Style::default().fg(theme::RED).bg(theme::BG))
                .wrap(Wrap { trim: true }),
            list_area,
        );
        return;
    }
    if app.mod_search_results.is_empty() {
        f.render_widget(
            Paragraph::new(
                "No results. Try a different search term, or clear the field and press Enter for popular picks.",
            )
            .style(theme::dim())
            .wrap(Wrap { trim: true }),
            list_area,
        );
        return;
    }

    // Build the visible list, applying the "installed only" filter when active.
    // Keeping original indices so click handlers still address the right hit.
    let visible: Vec<(usize, SearchHit)> = if app.installed_filter_only {
        app.mod_search_results
            .iter()
            .enumerate()
            .filter(|(_, h)| app.is_project_installed(&h.project_id))
            .map(|(i, h)| (i, h.clone()))
            .collect()
    } else {
        app.mod_search_results
            .iter()
            .enumerate()
            .map(|(i, h)| (i, h.clone()))
            .collect()
    };

    // "Show more" only makes sense when not filtering locally to installed.
    let has_more = !app.installed_filter_only
        && (app.mod_search_results.len() as u32) < app.mod_search_total;
    let footer_h: u16 = if has_more { 1 } else { 0 };
    let usable_h = list_area.height.saturating_sub(footer_h);

    let h = (usable_h as usize) / 2; // each row is 2 lines tall
    let total = visible.len();
    let start = app.mod_search_offset.min(total);
    let end = (start + h).min(total);
    for (i, (global, hit)) in visible[start..end].iter().enumerate() {
        let global = *global;
        let y = list_area.y + (i as u16) * 2;
        if y + 1 >= list_area.y + usable_h {
            break;
        }
        let row_rect = Rect::new(list_area.x, y, list_area.width, 2);
        let installed = app.is_project_installed(&hit.project_id);
        let installing_this = app.mod_installing.as_deref() == Some(hit.project_id.as_str());
        let hovered = app.hover == Some(Hit::ModResult(global)) && !installed;
        let bg = if hovered { theme::PANEL_HI } else { theme::BG };
        let (title_fg, body_fg, dim_fg) = if installed {
            (theme::FG_DIM, theme::FG_DIM, theme::FG_DIM)
        } else {
            (theme::ACCENT_HI, theme::FG, theme::FG_DIM)
        };
        let title_style = Style::default()
            .fg(title_fg)
            .bg(bg)
            .add_modifier(if installed { Modifier::empty() } else { Modifier::BOLD });
        let dim_style = Style::default().fg(dim_fg).bg(bg);
        let prefix = if installed {
            "✓ "
        } else if installing_this {
            "⏳ "
        } else {
            "▸ "
        };
        let prefix_color = if installed { theme::ACCENT } else { theme::ACCENT };
        let l1 = Line::from(vec![
            Span::styled(prefix, Style::default().fg(prefix_color).bg(bg)),
            Span::styled(hit.title.clone(), title_style),
            Span::styled(format!("  by {}", hit.author), dim_style),
            if installed {
                Span::styled("   (installed)", Style::default().fg(theme::ACCENT).bg(bg))
            } else {
                Span::raw("")
            },
        ]);
        let desc = if hit.description.len() > 100 {
            format!("{}...", &hit.description[..97])
        } else {
            hit.description.clone()
        };
        let l2 = Line::from(vec![
            Span::styled("    ", Style::default().bg(bg)),
            Span::styled(desc, Style::default().fg(body_fg).bg(bg)),
        ]);
        f.render_widget(
            Paragraph::new(vec![l1, l2]).style(Style::default().bg(bg)),
            row_rect,
        );
        if !installed {
            app.click_regions.push((row_rect, Hit::ModResult(global)));
        }
    }

    if has_more {
        let footer_y = list_area.y + usable_h;
        let footer_rect = Rect::new(list_area.x, footer_y, list_area.width, 1);
        let hovered = app.hover == Some(Hit::ShowMoreModsButton);
        let label = if app.mod_search_loading {
            "Loading more...".to_string()
        } else {
            format!(
                "↓ Show more ({} of {})",
                app.mod_search_results.len(),
                app.mod_search_total
            )
        };
        let style = if hovered {
            Style::default()
                .fg(theme::ACCENT_HI)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(theme::ACCENT)
                .bg(theme::BG)
                .add_modifier(Modifier::UNDERLINED)
        };
        f.render_widget(
            Paragraph::new(label).style(style).alignment(Alignment::Center),
            footer_rect,
        );
        if !app.mod_search_loading {
            app.click_regions.push((footer_rect, Hit::ShowMoreModsButton));
        }
    }
}

fn pretty_category(name: &str) -> String {
    // Modrinth slugs are kebab-case lowercase; Title-Case the words for display.
    let mut out = String::with_capacity(name.len());
    for (i, part) in name.split('-').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            for c in first.to_uppercase() {
                out.push(c);
            }
            out.extend(chars);
        }
    }
    out
}

fn draw_mod_search_field(f: &mut Frame, app: &mut App, rect: Rect) {
    let focused = app.focus == Focus::ModSearch;
    let hovered = app.hover == Some(Hit::ModSearchField);
    let border_fg = if focused {
        theme::ACCENT
    } else if hovered {
        theme::FG_DIM
    } else {
        theme::ACCENT
    };
    let title = match app.browser_kind {
        ContentKind::Mods => " 🔍 Filter mods ",
        ContentKind::Shaders => " 🔍 Filter shaders ",
        ContentKind::ResourcePacks => " 🔍 Filter texture packs ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_fg).bg(theme::BG))
        .style(theme::base())
        .title(Span::styled(title, theme::accent_bold()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let placeholder = match app.browser_kind {
        ContentKind::Mods => "Type to filter mods, then press Enter",
        ContentKind::Shaders => "Type to filter shader packs, then press Enter",
        ContentKind::ResourcePacks => "Type to filter texture packs, then press Enter",
    };
    let content = if focused {
        format!("{}▎", app.mod_search_query)
    } else if app.mod_search_query.is_empty() {
        placeholder.to_string()
    } else {
        app.mod_search_query.clone()
    };
    let style = if !focused && app.mod_search_query.is_empty() {
        theme::dim()
    } else {
        theme::base()
    };
    draw_vcentered_label(f, &format!(" {content}"), inner, style);
    app.click_regions.push((rect, Hit::ModSearchField));
}

fn draw_installed_mods_pane(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(theme::base())
        .title(Span::styled(" Installed ", theme::accent_bold()));
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    f.render_widget(block, area);

    if app.installed_mods.is_empty() {
        f.render_widget(
            Paragraph::new("No mods installed yet.\n\nSearch on the left and click a result.")
                .style(theme::dim())
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let h = inner.height as usize;
    let total = app.installed_mods.len();
    let visible = total.min(h);
    for (i, name) in app.installed_mods.iter().take(visible).enumerate() {
        let y = inner.y + i as u16;
        let row_rect = Rect::new(inner.x, y, inner.width, 1);
        let hovered = app.hover == Some(Hit::RemoveModButton(i));
        let style = if hovered {
            Style::default().fg(theme::RED).bg(theme::BG)
        } else {
            Style::default().fg(theme::FG).bg(theme::BG)
        };
        let label = if hovered {
            format!("✗  {name}")
        } else {
            format!("•  {name}")
        };
        let truncated = if label.chars().count() > row_rect.width as usize {
            label
                .chars()
                .take(row_rect.width as usize)
                .collect::<String>()
        } else {
            label
        };
        f.render_widget(Paragraph::new(truncated).style(style), row_rect);
        app.click_regions
            .push((row_rect, Hit::RemoveModButton(i)));
    }
}

fn draw_update_modal(f: &mut Frame, app: &mut App, area: Rect) {
    let w = 60u16.min(area.width.saturating_sub(4));
    let h = 11u16.min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);

    // Dim the surrounding frame a touch so the modal pops.
    f.render_widget(
        Fill {
            style: Style::default().fg(theme::FG_DIM).bg(theme::BG),
        },
        rect,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::GOLD))
        .style(theme::base())
        .title(Span::styled(
            " Update available ",
            Style::default()
                .fg(theme::GOLD)
                .bg(theme::BG)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(rect).inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    f.render_widget(block, rect);

    let (info, downloading, done, total, ready, asset_missing) = match &app.update_status {
        UpdateStatus::Downloading { info, done, total } => {
            (info.clone(), true, *done, *total, false, false)
        }
        UpdateStatus::Ready { info, .. } => (info.clone(), false, 0, 0, true, false),
        UpdateStatus::Outdated(info) => (info.clone(), false, 0, 0, false, true),
        _ => return,
    };

    let mut y = inner.y;
    let row = |yy: u16| Rect::new(inner.x, yy, inner.width, 1);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Current: ", theme::dim()),
            Span::styled(format!("v{}", info.current), theme::base()),
            Span::styled("    ", theme::base()),
            Span::styled("Latest: ", theme::dim()),
            Span::styled(
                format!("v{}", info.latest),
                Style::default()
                    .fg(theme::ACCENT_HI)
                    .bg(theme::BG)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .style(theme::base()),
        row(y),
    );
    y += 2;

    if downloading {
        let pct = if total == 0 {
            0.0
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        };
        let width = inner.width.saturating_sub(8) as usize;
        let filled = (pct * width as f64).round() as usize;
        let bar: String = std::iter::repeat('█')
            .take(filled)
            .chain(std::iter::repeat('░').take(width.saturating_sub(filled)))
            .collect();
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(bar, Style::default().fg(theme::ACCENT)),
                Span::raw(" "),
                Span::styled(format!("{:.0}%", pct * 100.0), theme::accent_bold()),
            ]))
            .style(theme::base()),
            row(y),
        );
        y += 1;
        let pretty = format!(
            "Downloading {} ({} / {})",
            info.asset.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
            human_bytes(done),
            human_bytes(total),
        );
        f.render_widget(Paragraph::new(pretty).style(theme::dim()), row(y));
        y += 2;
    } else if ready {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "The new version is downloaded and ready to install.",
                Style::default().fg(theme::ACCENT_HI).bg(theme::BG),
            )))
            .style(theme::base())
            .wrap(Wrap { trim: true }),
            Rect::new(inner.x, y, inner.width, 2),
        );
        y += 3;
    } else if asset_missing {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No prebuilt binary for this platform on the release yet. Open the release page to grab it manually.",
                Style::default().fg(theme::GOLD).bg(theme::BG),
            )))
            .style(theme::base())
            .wrap(Wrap { trim: true }),
            Rect::new(inner.x, y, inner.width, 3),
        );
        y += 4;
    }

    // Buttons row
    let btn_y = inner.y + inner.height.saturating_sub(BUTTON_H);
    let btn_row = Rect::new(inner.x, btn_y, inner.width, BUTTON_H);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(20),
            Constraint::Length(2),
            Constraint::Length(12),
        ])
        .split(btn_row);
    let _ = y;
    if ready {
        draw_button(f, app, cols[1], "Install & restart", Hit::InstallUpdateNow, true);
    } else if downloading {
        draw_disabled_button(f, cols[1], "Downloading...");
    } else if asset_missing {
        draw_button(f, app, cols[1], "Open releases page", Hit::OpenReleasesPage, true);
    }
    draw_button(f, app, cols[3], "Later", Hit::DismissUpdate, false);
}

fn human_bytes(n: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if n >= GIB {
        format!("{:.2} GiB", n as f64 / GIB as f64)
    } else if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.0} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

pub fn hit_test(app: &App, col: u16, row: u16) -> Option<Hit> {
    hit_region(app, col, row).map(|(_, hit)| hit)
}

pub fn hit_region(app: &App, col: u16, row: u16) -> Option<(Rect, Hit)> {
    for (rect, hit) in app.click_regions.iter().rev() {
        if col >= rect.x
            && col < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
        {
            return Some((*rect, *hit));
        }
    }
    None
}
