use ratatui::style::{Color, Modifier, Style};

pub const BG_BASE: Color = Color::Rgb(0x1A, 0x1B, 0x26);
pub const BG_SURFACE: Color = Color::Rgb(0x24, 0x25, 0x35);
pub const BG_OVERLAY: Color = Color::Rgb(0x2F, 0x30, 0x44);

pub const FG: Color = Color::Rgb(0xC8, 0xC8, 0xD4);
pub const FG_DIM: Color = Color::Rgb(0x6C, 0x6E, 0x7E);
pub const FG_BRIGHT: Color = Color::Rgb(0xE8, 0xE8, 0xF0);

pub const ACCENT: Color = Color::Rgb(0x7C, 0x6B, 0xFF);
pub const SUCCESS: Color = Color::Rgb(0x12, 0xC7, 0x8F);
pub const WARNING: Color = Color::Rgb(0xE8, 0xC5, 0x47);
pub const ERROR: Color = Color::Rgb(0xFF, 0x57, 0x7D);
pub const HIGHLIGHT: Color = Color::Rgb(0xFF, 0x4F, 0xBF);

pub fn base() -> Style {
    Style::default().bg(BG_BASE).fg(FG)
}

pub fn surface() -> Style {
    Style::default().bg(BG_SURFACE).fg(FG)
}

pub fn tab_active() -> Style {
    Style::default()
        .bg(ACCENT)
        .fg(FG_BRIGHT)
        .add_modifier(Modifier::BOLD)
}

pub fn tab_inactive() -> Style {
    Style::default().bg(BG_BASE).fg(FG_DIM)
}

pub fn table_header() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn table_selected() -> Style {
    Style::default().bg(BG_OVERLAY).fg(HIGHLIGHT)
}

pub fn status_bar() -> Style {
    Style::default().bg(BG_OVERLAY).fg(FG)
}

pub fn status_key() -> Style {
    Style::default()
        .bg(BG_OVERLAY)
        .fg(ACCENT)
        .add_modifier(Modifier::BOLD)
}

pub fn status_desc() -> Style {
    Style::default().bg(BG_OVERLAY).fg(FG_DIM)
}

pub fn input_active() -> Style {
    Style::default().bg(BG_SURFACE).fg(FG_BRIGHT)
}

pub fn input_placeholder() -> Style {
    Style::default().fg(FG_DIM)
}

pub fn border_active() -> Style {
    Style::default().fg(ACCENT)
}

pub fn border_dim() -> Style {
    Style::default().fg(Color::Rgb(0x3A, 0x3B, 0x50))
}

pub fn progress_gauge() -> Style {
    Style::default().fg(SUCCESS).bg(BG_SURFACE)
}

pub fn error_style() -> Style {
    Style::default().fg(ERROR)
}

pub fn success_style() -> Style {
    Style::default().fg(SUCCESS)
}

pub fn warning_style() -> Style {
    Style::default().fg(WARNING)
}

pub fn label() -> Style {
    Style::default().fg(FG_DIM)
}

pub fn value() -> Style {
    Style::default().fg(FG)
}

pub fn title() -> Style {
    Style::default().fg(FG_BRIGHT).add_modifier(Modifier::BOLD)
}
