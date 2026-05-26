#![allow(dead_code)]

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(18, 20, 24);
pub const PANEL: Color = Color::Rgb(28, 32, 38);
pub const PANEL_HI: Color = Color::Rgb(40, 46, 54);
pub const BORDER: Color = Color::Rgb(60, 68, 78);
pub const BORDER_HI: Color = Color::Rgb(120, 200, 140);

pub const FG: Color = Color::Rgb(220, 224, 230);
pub const FG_DIM: Color = Color::Rgb(140, 148, 160);
pub const ACCENT: Color = Color::Rgb(120, 200, 140);
pub const ACCENT_HI: Color = Color::Rgb(160, 230, 170);
pub const GOLD: Color = Color::Rgb(238, 196, 88);
pub const RED: Color = Color::Rgb(220, 96, 96);

pub fn base() -> Style {
    Style::default().fg(FG).bg(BG)
}

pub fn panel() -> Style {
    Style::default().fg(FG).bg(PANEL)
}

pub fn dim() -> Style {
    Style::default().fg(FG_DIM).bg(BG)
}

pub fn accent_bold() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn button_idle() -> Style {
    Style::default().fg(FG).bg(PANEL_HI)
}

pub fn button_hover() -> Style {
    Style::default()
        .fg(BG)
        .bg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn button_primary() -> Style {
    Style::default()
        .fg(BG)
        .bg(GOLD)
        .add_modifier(Modifier::BOLD)
}

pub fn list_selected() -> Style {
    Style::default()
        .fg(ACCENT_HI)
        .bg(PANEL_HI)
        .add_modifier(Modifier::BOLD)
}
