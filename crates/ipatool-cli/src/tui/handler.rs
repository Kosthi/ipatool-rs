use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::backend::crossterm::EventHandler;

use super::action::Action;
use super::app::{ActiveTab, App_, InputMode};

pub fn handle_key(app: &mut App_, key: KeyEvent) {
    if let InputMode::Popup(_) = &app.input_mode {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                app.input_mode = InputMode::Normal;
            }
            _ => {}
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.action_tx.send(Action::Quit).ok();
        return;
    }

    match &app.input_mode {
        InputMode::SearchInput => handle_search_input(app, key),
        InputMode::LoginEmail => handle_login_email(app, key),
        InputMode::LoginPassword => handle_login_password(app, key),
        InputMode::LoginAuthCode => handle_login_auth_code(app, key),
        InputMode::Normal => handle_normal(app, key),
        InputMode::Popup(_) => unreachable!(),
    }
}

fn handle_normal(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') => {
            app.action_tx.send(Action::Quit).ok();
        }
        KeyCode::Tab => {
            let next = match app.active_tab {
                ActiveTab::Search => ActiveTab::Library,
                ActiveTab::Library => ActiveTab::Downloads,
                ActiveTab::Downloads => ActiveTab::Account,
                ActiveTab::Account => ActiveTab::Search,
            };
            app.action_tx.send(Action::SwitchTab(next)).ok();
        }
        KeyCode::BackTab => {
            let prev = match app.active_tab {
                ActiveTab::Search => ActiveTab::Account,
                ActiveTab::Library => ActiveTab::Search,
                ActiveTab::Downloads => ActiveTab::Library,
                ActiveTab::Account => ActiveTab::Downloads,
            };
            app.action_tx.send(Action::SwitchTab(prev)).ok();
        }
        KeyCode::Char('1') => {
            app.action_tx
                .send(Action::SwitchTab(ActiveTab::Search))
                .ok();
        }
        KeyCode::Char('2') => {
            app.action_tx
                .send(Action::SwitchTab(ActiveTab::Library))
                .ok();
        }
        KeyCode::Char('3') => {
            app.action_tx
                .send(Action::SwitchTab(ActiveTab::Downloads))
                .ok();
        }
        KeyCode::Char('4') => {
            app.action_tx
                .send(Action::SwitchTab(ActiveTab::Account))
                .ok();
        }
        _ => match app.active_tab {
            ActiveTab::Search => handle_search_normal(app, key),
            ActiveTab::Downloads => handle_downloads_normal(app, key),
            ActiveTab::Account => handle_account_normal(app, key),
            ActiveTab::Library => {}
        },
    }
}

fn handle_search_normal(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Char('/') | KeyCode::Char('s') => {
            app.input_mode = InputMode::SearchInput;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !app.search_results.is_empty() {
                let i = app.search_table_state.selected().map_or(0, |i| {
                    if i >= app.search_results.len() - 1 {
                        0
                    } else {
                        i + 1
                    }
                });
                app.search_table_state.select(Some(i));
                app.update_selected_detail();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if !app.search_results.is_empty() {
                let i = app.search_table_state.selected().map_or(0, |i| {
                    if i == 0 {
                        app.search_results.len() - 1
                    } else {
                        i - 1
                    }
                });
                app.search_table_state.select(Some(i));
                app.update_selected_detail();
            }
        }
        KeyCode::Char('d') => {
            if let Some(selected) = app.selected_app() {
                app.action_tx
                    .send(Action::StartDownload {
                        bundle_id: selected.bundle_id.clone(),
                        app_name: selected.name.clone(),
                        app_id: selected.id,
                    })
                    .ok();
            }
        }
        KeyCode::Char('p') => {
            if let Some(selected) = app.selected_app() {
                app.action_tx
                    .send(Action::Purchase(selected.id, selected.name.clone()))
                    .ok();
            }
        }
        _ => {}
    }
}

fn handle_downloads_normal(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            if !app.downloads.is_empty() {
                let i = app.download_list_state.selected().map_or(0, |i| {
                    if i >= app.downloads.len() - 1 {
                        0
                    } else {
                        i + 1
                    }
                });
                app.download_list_state.select(Some(i));
            }
        }
        KeyCode::Up | KeyCode::Char('k') if !app.downloads.is_empty() => {
            let i = app.download_list_state.selected().map_or(0, |i| {
                if i == 0 {
                    app.downloads.len() - 1
                } else {
                    i - 1
                }
            });
            app.download_list_state.select(Some(i));
        }
        KeyCode::Char('x') => {
            if let Some(idx) = app.download_list_state.selected() {
                if let Some(dl) = app.downloads.get(idx) {
                    app.action_tx.send(Action::CancelDownload(dl.id)).ok();
                }
            }
        }
        KeyCode::Char('c') => {
            app.action_tx.send(Action::ClearFinishedDownloads).ok();
        }
        _ => {}
    }
}

fn handle_account_normal(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Char('l') => {
            if app.account.is_none() {
                app.input_mode = InputMode::LoginEmail;
                app.login_error = None;
                app.login_auth_code = tui_input::Input::default();
                app.login_needs_auth_code = false;
            }
        }
        KeyCode::Char('r') if app.account.is_some() => {
            app.action_tx.send(Action::Logout).ok();
        }
        _ => {}
    }
}

fn handle_search_input(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            app.action_tx.send(Action::SubmitSearch).ok();
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        _ => {
            app.search_input
                .handle_event(&crossterm::event::Event::Key(key));
        }
    }
}

fn handle_login_email(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Enter | KeyCode::Tab => {
            app.input_mode = InputMode::LoginPassword;
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        _ => {
            app.login_email
                .handle_event(&crossterm::event::Event::Key(key));
        }
    }
}

fn handle_login_password(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            if app.login_needs_auth_code {
                app.input_mode = InputMode::LoginAuthCode;
            } else {
                app.input_mode = InputMode::Normal;
                app.action_tx.send(Action::SubmitLogin).ok();
            }
        }
        KeyCode::Tab if app.login_needs_auth_code => {
            app.input_mode = InputMode::LoginAuthCode;
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Backspace => {
            app.login_password.pop();
        }
        KeyCode::Char(c) => {
            app.login_password.push(c);
        }
        _ => {}
    }
}

fn handle_login_auth_code(app: &mut App_, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            app.action_tx.send(Action::SubmitLogin).ok();
        }
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        _ => {
            app.login_auth_code
                .handle_event(&crossterm::event::Event::Key(key));
        }
    }
}
