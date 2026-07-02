use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tui_input::backend::crossterm::EventHandler;

use super::action::Action;
use super::app::{ActiveTab, App_, InputMode};

pub fn handle_key(app: &mut App_, key: KeyEvent) {
    if key.kind == KeyEventKind::Release {
        return;
    }

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
            if let Some(idx) = app.download_list_state.selected()
                && let Some(dl) = app.downloads.get(idx)
            {
                app.action_tx.send(Action::CancelDownload(dl.id)).ok();
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
        KeyCode::Enter | KeyCode::Tab | KeyCode::Down => {
            app.input_mode = InputMode::LoginPassword;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.input_mode = InputMode::LoginAuthCode;
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
            app.input_mode = InputMode::Normal;
            app.action_tx.send(Action::SubmitLogin).ok();
        }
        KeyCode::Tab | KeyCode::Down => {
            app.input_mode = InputMode::LoginAuthCode;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.input_mode = InputMode::LoginEmail;
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
        KeyCode::Tab | KeyCode::Down => {
            app.input_mode = InputMode::LoginEmail;
        }
        KeyCode::BackTab | KeyCode::Up => {
            app.input_mode = InputMode::LoginPassword;
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use ratatui::widgets::{ListState, TableState};
    use tokio::sync::mpsc::{self, UnboundedReceiver};
    use tokio::sync::{Mutex, Semaphore};
    use tui_input::Input;

    use ipatool_core::client::AppleClient;
    use ipatool_core::model::Platform;

    use super::*;
    use crate::tui::app::{ActiveTab, App_, InputMode};

    fn test_app_with_rx() -> (App_, UnboundedReceiver<Action>) {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let client = AppleClient::new("test-guid".to_string(), None).unwrap();

        let app = App_ {
            active_tab: ActiveTab::Search,
            input_mode: InputMode::Normal,
            should_quit: false,
            action_tx,
            search_input: Input::default(),
            search_results: Vec::new(),
            search_table_state: TableState::default(),
            search_platform: Platform::IPhone,
            search_country: "US".to_string(),
            is_loading: false,
            selected_detail: None,
            downloads: Vec::new(),
            download_list_state: ListState::default(),
            download_semaphore: Arc::new(Semaphore::new(3)),
            next_download_id: 0,
            account: None,
            login_email: Input::default(),
            login_password: String::new(),
            login_auth_code: Input::default(),
            login_error: None,
            client: Arc::new(Mutex::new(client)),
            status_message: String::new(),
        };

        (app, action_rx)
    }

    fn test_app() -> App_ {
        test_app_with_rx().0
    }

    fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
    }

    #[test]
    fn ignores_key_release_events() {
        let mut app = test_app();
        app.input_mode = InputMode::LoginPassword;

        handle_key(&mut app, key(KeyCode::Char('p'), KeyEventKind::Press));
        handle_key(&mut app, key(KeyCode::Char('p'), KeyEventKind::Release));

        assert_eq!(app.login_password, "p");
    }

    #[test]
    fn preserves_key_repeat_events() {
        let mut app = test_app();
        app.input_mode = InputMode::LoginPassword;

        handle_key(&mut app, key(KeyCode::Char('p'), KeyEventKind::Press));
        handle_key(&mut app, key(KeyCode::Char('p'), KeyEventKind::Repeat));

        assert_eq!(app.login_password, "pp");
    }

    #[test]
    fn ignores_key_release_events_for_navigation() {
        let (mut app, mut action_rx) = test_app_with_rx();

        handle_key(&mut app, key(KeyCode::Tab, KeyEventKind::Press));
        handle_key(&mut app, key(KeyCode::Tab, KeyEventKind::Release));

        match action_rx.try_recv() {
            Ok(Action::SwitchTab(ActiveTab::Library)) => {}
            other => panic!("expected one switch-tab action, got {other:?}"),
        }
        assert!(action_rx.try_recv().is_err());
    }

    #[test]
    fn arrow_keys_move_login_field_focus() {
        let mut app = test_app();
        app.input_mode = InputMode::LoginEmail;

        handle_key(&mut app, key(KeyCode::Down, KeyEventKind::Press));
        assert_eq!(app.input_mode, InputMode::LoginPassword);

        handle_key(&mut app, key(KeyCode::Down, KeyEventKind::Press));
        assert_eq!(app.input_mode, InputMode::LoginAuthCode);

        handle_key(&mut app, key(KeyCode::Up, KeyEventKind::Press));
        assert_eq!(app.input_mode, InputMode::LoginPassword);

        handle_key(&mut app, key(KeyCode::Up, KeyEventKind::Press));
        assert_eq!(app.input_mode, InputMode::LoginEmail);
    }
}
