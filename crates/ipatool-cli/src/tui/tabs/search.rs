use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};

use crate::tui::app::{App_, InputMode};
use crate::tui::format_bytes;
use crate::tui::theme;

pub fn render(f: &mut Frame, app: &mut App_, area: Rect) {
    let bg = Block::default().style(theme::base());
    f.render_widget(bg, area);

    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(5)]).split(area);

    render_search_bar(f, app, chunks[0]);
    render_content(f, app, chunks[1]);
}

fn render_search_bar(f: &mut Frame, app: &App_, area: Rect) {
    let is_active = app.input_mode == InputMode::SearchInput;

    let input_text = app.search_input.value();
    let display = if input_text.is_empty() && !is_active {
        Line::from(Span::styled(
            "  Search apps... (press /)",
            theme::input_placeholder(),
        ))
    } else {
        Line::from(vec![
            Span::styled("  ", theme::label()),
            Span::styled(input_text, theme::input_active()),
        ])
    };

    let border_style = if is_active {
        theme::border_active()
    } else {
        theme::border_dim()
    };

    let input = Paragraph::new(display).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(border_style),
    );
    f.render_widget(input, area);

    if is_active {
        let cursor_x = area.x + app.search_input.visual_cursor() as u16 + 3;
        let cursor_y = area.y;
        f.set_cursor_position((cursor_x, cursor_y));
    }
}

fn render_content(f: &mut Frame, app: &mut App_, area: Rect) {
    if app.search_results.is_empty() {
        let msg = if app.is_loading {
            vec![
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  Searching...", theme::warning_style())),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  ipatool", theme::title())),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press / to search for iOS apps",
                    theme::label(),
                )),
                Line::from(Span::styled(
                    "  Then d to download, p to purchase",
                    theme::label(),
                )),
            ]
        };
        let p = Paragraph::new(msg).alignment(Alignment::Left);
        f.render_widget(p, area);
        return;
    }

    let chunks =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

    render_table(f, app, chunks[0]);
    render_detail(f, app, chunks[1]);
}

fn render_table(f: &mut Frame, app: &mut App_, area: Rect) {
    let header = Row::new(vec![
        Cell::from("  Name"),
        Cell::from("Bundle ID"),
        Cell::from("Ver"),
        Cell::from("Price"),
    ])
    .style(theme::table_header())
    .height(1);

    let rows: Vec<Row> = app
        .search_results
        .iter()
        .map(|a| {
            let price = if a.price == 0.0 {
                "Free".to_string()
            } else {
                format!("${:.2}", a.price)
            };
            Row::new(vec![
                Cell::from(format!("  {}", a.name)),
                Cell::from(a.bundle_id.clone()),
                Cell::from(a.version.clone().unwrap_or_default()),
                Cell::from(price),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(35),
            Constraint::Percentage(35),
            Constraint::Percentage(15),
            Constraint::Percentage(15),
        ],
    )
    .header(header)
    .row_highlight_style(theme::table_selected())
    .highlight_symbol(" > ");

    f.render_stateful_widget(table, area, &mut app.search_table_state);
}

fn render_detail(f: &mut Frame, app: &App_, area: Rect) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(theme::border_dim());

    let Some(selected) = &app.selected_detail else {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  Select an app", theme::label())),
        ])
        .block(block);
        f.render_widget(p, area);
        return;
    };

    let price = if selected.price == 0.0 {
        "Free".to_string()
    } else {
        format!("${:.2}", selected.price)
    };

    let artist = selected
        .extra
        .get("artistName")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");

    let genre = selected
        .extra
        .get("primaryGenreName")
        .and_then(|v| v.as_str())
        .unwrap_or("—");

    let rating = selected
        .extra
        .get("averageUserRating")
        .and_then(|v| v.as_f64())
        .map(|r| format!("{r:.1}"))
        .unwrap_or_else(|| "—".to_string());

    let size = selected
        .extra
        .get("fileSizeBytes")
        .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")))
        .and_then(|s| {
            if s.is_empty() {
                selected
                    .extra
                    .get("fileSizeBytes")
                    .and_then(|v| v.as_u64())
                    .map(format_bytes)
            } else {
                s.parse::<u64>().ok().map(format_bytes)
            }
        })
        .unwrap_or_else(|| "—".to_string());

    let app_id_str = selected.id.to_string();
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("  {}", selected.name), theme::title())),
        Line::from(Span::styled(format!("  {artist}"), theme::label())),
        Line::from(""),
        detail_line("  Bundle", &selected.bundle_id),
        detail_line("  Version", selected.version.as_deref().unwrap_or("—")),
        detail_line("  ID", &app_id_str),
        detail_line("  Price", &price),
        detail_line("  Genre", genre),
        detail_line("  Rating", &rating),
        detail_line("  Size", &size),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  d",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" download  ", theme::label()),
            Span::styled(
                "p",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" purchase", theme::label()),
        ]),
    ];

    let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn detail_line<'a>(label: &'a str, value: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label}: "), theme::label()),
        Span::styled(value, theme::value()),
    ])
}
