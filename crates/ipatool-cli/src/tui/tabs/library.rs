use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::tui::app::App_;
use crate::tui::theme;

pub fn render(f: &mut Frame, app: &mut App_, area: Rect) {
    let bg = Block::default().style(theme::base());
    f.render_widget(bg, area);

    let text = if app.account.is_some() {
        vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  Library", theme::title())),
            Line::from(""),
            Line::from(Span::styled(
                "  Purchase history API not yet implemented.",
                theme::label(),
            )),
            Line::from(Span::styled(
                "  Use Search tab to find and download apps.",
                theme::label(),
            )),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  Library", theme::title())),
            Line::from(""),
            Line::from(Span::styled(
                "  Log in on the Account tab to view your library.",
                theme::label(),
            )),
        ]
    };

    f.render_widget(Paragraph::new(text), area);
}
