use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::tui::app::{App_, DownloadStage};
use crate::tui::format_bytes;
use crate::tui::theme;

pub fn render(f: &mut Frame, app: &mut App_, area: Rect) {
    let bg = Block::default().style(theme::base());
    f.render_widget(bg, area);

    if app.downloads.is_empty() {
        let text = vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("  Downloads", theme::title())),
            Line::from(""),
            Line::from(Span::styled("  No active downloads.", theme::label())),
            Line::from(Span::styled(
                "  Search for an app and press d to download.",
                theme::label(),
            )),
        ];
        f.render_widget(Paragraph::new(text), area);
        return;
    }

    let items: Vec<ListItem> = app
        .downloads
        .iter()
        .map(|dl| {
            let available_width = area.width.saturating_sub(4) as usize;

            match dl.stage {
                DownloadStage::Downloading if dl.total > 0 => {
                    let ratio = dl.progress as f64 / dl.total as f64;
                    let pct = (ratio * 100.0) as u64;

                    let bar_width = 20usize.min(available_width / 3);
                    let filled = (ratio * bar_width as f64) as usize;
                    let empty = bar_width.saturating_sub(filled);
                    let bar = format!(
                        "{}{}",
                        "█".repeat(filled),
                        "░".repeat(empty),
                    );

                    let line = Line::from(vec![
                        Span::styled(format!("  {}", dl.app_name), theme::value()),
                        Span::styled("  ", theme::label()),
                        Span::styled(bar, theme::progress_gauge()),
                        Span::styled(
                            format!(
                                "  {} / {} ({}%)",
                                format_bytes(dl.progress),
                                format_bytes(dl.total),
                                pct
                            ),
                            theme::label(),
                        ),
                    ]);

                    ListItem::new(line)
                }
                DownloadStage::Failed => {
                    let header = Line::from(vec![
                        Span::styled(format!("  {}", dl.app_name), theme::value()),
                        Span::styled(" — ", theme::label()),
                        Span::styled("Failed", theme::error_style()),
                    ]);
                    if let Some(ref err) = dl.error {
                        ListItem::new(vec![
                            header,
                            Line::from(Span::styled(
                                format!("  {err}"),
                                theme::error_style(),
                            )),
                        ])
                    } else {
                        ListItem::new(header)
                    }
                }
                _ => {
                    let status_style = match dl.stage {
                        DownloadStage::Complete => theme::success_style(),
                        DownloadStage::Downloading
                        | DownloadStage::Purchasing
                        | DownloadStage::Patching => theme::warning_style(),
                        DownloadStage::Queued | DownloadStage::Cancelled => theme::label(),
                        _ => theme::label(),
                    };

                    let line = Line::from(vec![
                        Span::styled(format!("  {}", dl.app_name), theme::value()),
                        Span::styled(" — ", theme::label()),
                        Span::styled(dl.stage.to_string(), status_style),
                    ]);
                    ListItem::new(line)
                }
            }
        })
        .collect();

    let list = List::new(items)
        .highlight_style(theme::table_selected())
        .highlight_symbol(" ");

    f.render_stateful_widget(list, area, &mut app.download_list_state);
}
