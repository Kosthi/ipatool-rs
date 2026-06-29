pub mod account;
pub mod download;
pub mod library;
pub mod search;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::tui::app::{ActiveTab, App_};

pub fn render_tab(f: &mut Frame, app: &mut App_, area: Rect) {
    match app.active_tab {
        ActiveTab::Search => search::render(f, app, area),
        ActiveTab::Library => library::render(f, app, area),
        ActiveTab::Downloads => download::render(f, app, area),
        ActiveTab::Account => account::render(f, app, area),
    }
}
