pub mod action;
pub mod app;
pub mod event;
pub mod handler;
pub mod tabs;
pub mod theme;

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::Mutex;
use tokio::sync::mpsc;

use ipatool_core::client::AppleClient;

use tokio_util::sync::CancellationToken;

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;

use crate::data_dir;

use self::action::Action;
use self::app::{ActiveTab, App_, DownloadStage, InputMode};
use self::event::{Event, EventHandler};

pub async fn run() -> Result<()> {
    let guid_str = ipatool_core::guid::generate_guid().context("failed to generate GUID")?;
    let data_dir = data_dir();
    std::fs::create_dir_all(&data_dir).ok();
    let cookie_path = data_dir.join("cookies.json");

    let client =
        AppleClient::new(guid_str, Some(&cookie_path)).context("failed to create client")?;

    let (action_tx, action_rx) = mpsc::unbounded_channel::<Action>();
    let mut app = App_::new(client, action_tx.clone());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_loop(&mut terminal, &mut app, action_rx, &cookie_path).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App_,
    mut action_rx: mpsc::UnboundedReceiver<Action>,
    cookie_path: &std::path::Path,
) -> Result<()> {
    let mut events = EventHandler::new();
    let mut needs_redraw = true;

    loop {
        if app.should_quit {
            let client = app.client.lock().await;
            client.save_cookies(cookie_path).ok();
            break;
        }

        if needs_redraw {
            terminal.draw(|f| ui(f, app))?;
            needs_redraw = false;
        }

        tokio::select! {
            event = events.next() => {
                match event? {
                    Event::Key(key) => {
                        handler::handle_key(app, key);
                        needs_redraw = true;
                    }
                    Event::Tick => {}
                    Event::Render => {
                        needs_redraw = true;
                    }
                    Event::Resize(_, _) => {
                        needs_redraw = true;
                    }
                }
            }
            Some(action) = action_rx.recv() => {
                process_action(app, action).await;
                needs_redraw = true;
            }
        }
    }

    Ok(())
}

async fn process_action(app: &mut App_, action: Action) {
    match action {
        Action::Quit => {
            app.should_quit = true;
        }
        Action::Tick | Action::Render => {}
        Action::SwitchTab(tab) => {
            app.active_tab = tab;
        }
        Action::SubmitSearch => {
            let query = app.search_input.value().to_string();
            if query.is_empty() {
                return;
            }
            app.is_loading = true;
            app.search_results.clear();
            app.search_table_state.select(None);
            app.selected_detail = None;
            app.status_message = format!("Searching for \"{query}\"...");

            let client = Arc::clone(&app.client);
            let tx = app.action_tx.clone();
            let country = app.search_country.clone();
            let platform = app.search_platform;

            tokio::spawn(async move {
                let result = {
                    let c = client.lock().await;
                    ipatool_core::api::search::search(&c, &query, &country, platform, 25).await
                };
                match result {
                    Ok(apps) => {
                        tx.send(Action::SearchResults(apps)).ok();
                    }
                    Err(e) => {
                        tx.send(Action::SearchError(e.to_string())).ok();
                    }
                }
            });
        }
        Action::SearchResults(apps) => {
            app.is_loading = false;
            let count = apps.len();
            app.search_results = apps;
            if !app.search_results.is_empty() {
                app.search_table_state.select(Some(0));
                app.update_selected_detail();
            }
            app.status_message = format!("Found {count} results");
        }
        Action::SearchError(err) => {
            app.is_loading = false;
            app.status_message = format!("Search error: {err}");
        }
        Action::StartDownload {
            bundle_id,
            app_name,
            app_id,
        } => {
            if app.account.is_none() {
                app.status_message = "Login required to download".to_string();
                return;
            }

            let cancel_token = CancellationToken::new();
            let id = app.next_download_id;
            app.next_download_id += 1;
            app.downloads.push(app::DownloadTask {
                id,
                app_name: app_name.clone(),
                stage: DownloadStage::Queued,
                progress: 0,
                total: 0,
                error: None,
                cancel_token: cancel_token.clone(),
            });
            app.download_list_state.select(Some(app.downloads.len() - 1));
            app.status_message = format!("Queued {app_name} — see Downloads tab");

            let client = Arc::clone(&app.client);
            let tx = app.action_tx.clone();
            let account = app.account.clone().unwrap();
            let semaphore = Arc::clone(&app.download_semaphore);

            tokio::spawn(async move {
                let _permit = match semaphore.acquire().await {
                    Ok(p) => p,
                    Err(_) => return,
                };
                if cancel_token.is_cancelled() {
                    tx.send(Action::DownloadCancelled(id)).ok();
                    return;
                }
                run_download_task(client, app_id, bundle_id, app_name, account, tx, id, cancel_token).await;
            });
        }
        Action::DownloadProgress {
            id,
            stage,
            progress,
            total,
        } => {
            if let Some(dl) = app.download_by_id_mut(id) {
                dl.stage = stage;
                dl.progress = progress;
                dl.total = total;
            }
        }
        Action::DownloadComplete(id) => {
            if let Some(dl) = app.download_by_id_mut(id) {
                dl.stage = DownloadStage::Complete;
            }
        }
        Action::DownloadError { id, error } => {
            if let Some(dl) = app.download_by_id_mut(id) {
                if dl.cancel_token.is_cancelled() {
                    dl.stage = DownloadStage::Cancelled;
                    app.status_message = "Download cancelled".to_string();
                } else {
                    dl.stage = DownloadStage::Failed;
                    dl.error = Some(error.clone());
                    app.status_message = format!("Download failed: {error}");
                }
            }
        }
        Action::CancelDownload(id) => {
            if let Some(dl) = app.download_by_id(id) {
                if !matches!(
                    dl.stage,
                    DownloadStage::Complete | DownloadStage::Failed | DownloadStage::Cancelled
                ) {
                    dl.cancel_token.cancel();
                }
            }
        }
        Action::DownloadCancelled(id) => {
            if let Some(dl) = app.download_by_id_mut(id) {
                dl.stage = DownloadStage::Cancelled;
            }
            app.status_message = "Download cancelled".to_string();
        }
        Action::ClearFinishedDownloads => {
            app.clear_finished_downloads();
            app.status_message = "Cleared finished downloads".to_string();
        }
        Action::SubmitLogin => {
            let email = app.login_email.value().to_string();
            let password = app.login_password.clone();
            if email.is_empty() || password.is_empty() {
                app.login_error = Some("Email and password required".to_string());
                return;
            }

            let auth_code = if app.login_needs_auth_code {
                let code = app.login_auth_code.value().trim().to_string();
                if code.is_empty() {
                    app.input_mode = InputMode::LoginAuthCode;
                    app.login_error = Some("Two-factor authentication code required".to_string());
                    return;
                }
                Some(code)
            } else {
                None
            };

            app.status_message = "Logging in...".to_string();
            app.login_error = None;

            let client = Arc::clone(&app.client);
            let tx = app.action_tx.clone();

            tokio::spawn(async move {
                let auth_url = {
                    let c = client.lock().await;
                    ipatool_core::api::bag::fetch_auth_endpoint(&c).await
                };

                let auth_url = match auth_url {
                    Ok(url) => url,
                    Err(e) => {
                        tx.send(Action::LoginError(e.to_string())).ok();
                        return;
                    }
                };

                let account = {
                    let c = client.lock().await;
                    ipatool_core::api::auth::login(
                        &c,
                        &email,
                        &password,
                        auth_code.as_deref(),
                        &auth_url,
                    )
                    .await
                };

                match account {
                    Ok(mut account) => {
                        account.password = Some(password);
                        if let Err(e) = ipatool_core::credential::store_account(&account) {
                            tx.send(Action::LoginError(format!("save failed: {e}")))
                                .ok();
                            return;
                        }
                        {
                            let mut c = client.lock().await;
                            c.set_account(account.clone());
                        }
                        tx.send(Action::LoginSuccess(account)).ok();
                    }
                    Err(e)
                        if matches!(
                            &e,
                            ipatool_core::error::ClientError::Store(
                                ipatool_core::error::StoreError::AuthCodeRequired
                            )
                        ) =>
                    {
                        tx.send(Action::LoginNeedsAuthCode).ok();
                    }
                    Err(e) => {
                        tx.send(Action::LoginError(e.to_string())).ok();
                    }
                }
            });
        }
        Action::LoginNeedsAuthCode => {
            app.login_needs_auth_code = true;
            app.login_auth_code = tui_input::Input::default();
            app.input_mode = InputMode::LoginAuthCode;
            app.login_error = Some("Two-factor authentication code required".to_string());
            app.status_message = "Enter two-factor authentication code".to_string();
        }
        Action::LoginSuccess(account) => {
            if let Some(country) =
                ipatool_core::model::storefront::country_code_from_store_front(&account.store_front)
            {
                app.search_country = country.to_string();
            }
            app.account = Some(account);
            app.login_password.clear();
            app.login_auth_code = tui_input::Input::default();
            app.login_needs_auth_code = false;
            app.status_message = "Logged in successfully".to_string();
        }
        Action::LoginError(err) => {
            if app.login_needs_auth_code {
                app.input_mode = InputMode::LoginAuthCode;
            }
            app.login_error = Some(err.clone());
            app.status_message = format!("Login failed: {err}");
        }
        Action::AccountRefreshed(account) => {
            app.account = Some(account);
        }
        Action::Logout => {
            if let Err(e) = ipatool_core::credential::delete_account() {
                app.status_message = format!("Logout failed: {e}");
            } else {
                app.account = None;
                app.login_auth_code = tui_input::Input::default();
                app.login_needs_auth_code = false;
                app.status_message = "Logged out".to_string();
            }
        }
        Action::Purchase(app_id, name) => {
            if app.account.is_none() {
                app.status_message = "Login required to purchase".to_string();
                return;
            }
            app.status_message = format!("Purchasing {name}...");

            let client = Arc::clone(&app.client);
            let tx = app.action_tx.clone();
            let account = app.account.clone().unwrap();

            tokio::spawn(async move {
                let result = {
                    let c = client.lock().await;
                    ipatool_core::api::purchase::purchase(&c, app_id, &account).await
                };
                match result {
                    Ok(()) => {
                        tx.send(Action::PurchaseSuccess(name)).ok();
                    }
                    Err(e) if e.is_token_expired() => {
                        let reauth_result = {
                            let c = client.lock().await;
                            tui_reauth(&c, &account).await
                        };
                        match reauth_result {
                            Ok(new_acc) => {
                                tx.send(Action::AccountRefreshed(new_acc.clone())).ok();
                                let mut c = client.lock().await;
                                c.set_account(new_acc.clone());
                                match ipatool_core::api::purchase::purchase(&c, app_id, &new_acc)
                                    .await
                                {
                                    Ok(()) => {
                                        tx.send(Action::PurchaseSuccess(name)).ok();
                                    }
                                    Err(e) => {
                                        tx.send(Action::PurchaseError(e.to_string())).ok();
                                    }
                                }
                            }
                            Err(e) => {
                                tx.send(Action::PurchaseError(format!("re-auth failed: {e}")))
                                    .ok();
                            }
                        }
                    }
                    Err(e) => {
                        tx.send(Action::PurchaseError(e.to_string())).ok();
                    }
                }
            });
        }
        Action::PurchaseSuccess(name) => {
            app.status_message = format!("Purchased: {name}");
        }
        Action::PurchaseError(err) => {
            app.status_message = format!("Purchase failed: {err}");
        }
        Action::ShowPopup(msg) => {
            app.input_mode = InputMode::Popup(msg);
        }
        Action::ClosePopup => {
            app.input_mode = InputMode::Normal;
        }
        Action::StatusMessage(msg) => {
            app.status_message = msg;
        }
    }
}

fn ui(f: &mut ratatui::Frame, app: &mut App_) {
    let bg = Block::default().style(theme::base());
    f.render_widget(bg, f.area());

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(f.area());

    render_tabs(f, app, chunks[0]);
    tabs::render_tab(f, app, chunks[1]);
    render_status_bar(f, app, chunks[2]);

    if let InputMode::Popup(ref msg) = app.input_mode {
        render_popup(f, msg);
    }
}

fn render_tabs(f: &mut ratatui::Frame, app: &App_, area: Rect) {
    let mut spans = vec![Span::styled(" ", theme::tab_inactive())];

    for tab in &ActiveTab::ALL {
        let label = format!(" {} ", tab.title());
        if *tab == app.active_tab {
            spans.push(Span::styled(label, theme::tab_active()));
        } else {
            spans.push(Span::styled(label, theme::tab_inactive()));
        }
        spans.push(Span::styled(" ", theme::tab_inactive()));
    }

    let country = &app.search_country;
    let account_hint = if app.account.is_some() {
        format!("[{country}] ")
    } else {
        "not logged in ".to_string()
    };
    let remaining = area
        .width
        .saturating_sub(spans.iter().map(|s| s.width() as u16).sum::<u16>());
    if remaining > account_hint.len() as u16 {
        let pad = remaining - account_hint.len() as u16;
        spans.push(Span::styled(
            " ".repeat(pad as usize),
            theme::tab_inactive(),
        ));
        spans.push(Span::styled(account_hint, theme::label()));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_status_bar(f: &mut ratatui::Frame, app: &App_, area: Rect) {
    let keys: &[(&str, &str)] = match app.input_mode {
        InputMode::Normal => match app.active_tab {
            ActiveTab::Search => &[
                ("q", "quit"),
                ("/", "search"),
                ("j/k", "nav"),
                ("d", "download"),
                ("p", "purchase"),
                ("Tab", "next"),
            ],
            ActiveTab::Library => &[("q", "quit"), ("Tab", "next")],
            ActiveTab::Downloads => &[("q", "quit"), ("j/k", "nav"), ("x", "cancel"), ("c", "clear"), ("Tab", "next")],
            ActiveTab::Account if app.account.is_some() => {
                &[("q", "quit"), ("r", "revoke"), ("Tab", "next")]
            }
            ActiveTab::Account => &[("q", "quit"), ("l", "login"), ("Tab", "next")],
        },
        InputMode::SearchInput => &[("Enter", "search"), ("Esc", "cancel")],
        InputMode::LoginEmail => &[("Tab", "next"), ("Esc", "cancel")],
        InputMode::LoginPassword => &[("Enter", "submit"), ("Esc", "cancel")],
        InputMode::LoginAuthCode => &[("Enter", "submit"), ("Esc", "cancel")],
        InputMode::Popup(_) => &[("Esc", "close")],
    };

    let mut spans = vec![Span::styled(
        format!(" {} ", app.status_message),
        theme::status_bar(),
    )];

    let status_len: usize = spans.iter().map(|s| s.width()).sum();
    let keys_len: usize = keys.iter().map(|(k, d)| k.len() + d.len() + 3).sum();
    let pad = area.width.saturating_sub((status_len + keys_len) as u16) as usize;
    spans.push(Span::styled(" ".repeat(pad), theme::status_bar()));

    for (key, desc) in keys {
        spans.push(Span::styled(format!(" {key}"), theme::status_key()));
        spans.push(Span::styled(format!(" {desc}"), theme::status_desc()));
    }

    let bar = Paragraph::new(Line::from(spans));
    f.render_widget(bar, area);
}

fn render_popup(f: &mut ratatui::Frame, msg: &str) {
    let area = f.area();
    let popup_width = (area.width * 60 / 100).max(30).min(area.width - 4);
    let popup_height = 7u16.min(area.height - 2);
    let x = (area.width - popup_width) / 2;
    let y = (area.height - popup_height) / 2;
    let popup_area = Rect::new(x, y, popup_width, popup_height);

    let popup = Paragraph::new(msg)
        .style(theme::surface())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::border_active())
                .title(" Info "),
        )
        .wrap(Wrap { trim: true });

    f.render_widget(ratatui::widgets::Clear, popup_area);
    f.render_widget(popup, popup_area);
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

async fn tui_reauth(
    client: &AppleClient,
    account: &ipatool_core::model::Account,
) -> Result<ipatool_core::model::Account> {
    let new_account = ipatool_core::api::reauth::reauthenticate(client, account)
        .await
        .context("re-authentication failed")?;
    ipatool_core::credential::store_account(&new_account)
        .context("failed to store refreshed credentials")?;
    Ok(new_account)
}

async fn run_download_task(
    client: Arc<Mutex<AppleClient>>,
    app_id: i64,
    bundle_id: String,
    app_name: String,
    account: ipatool_core::model::Account,
    tx: mpsc::UnboundedSender<Action>,
    id: usize,
    cancel_token: CancellationToken,
) {
    let mut account = account;

    let item = match acquire_download_item(&client, app_id, &mut account, &tx, id).await {
        Ok(item) => item,
        Err(e) => {
            tx.send(Action::DownloadError { id, error: e }).ok();
            return;
        }
    };

    let version = item
        .metadata
        .get("bundleShortVersionString")
        .and_then(|v| v.as_string())
        .unwrap_or("unknown");

    let filename = format!("{bundle_id}_{app_id}_{version}.ipa");
    let dest = PathBuf::from(&filename);
    let tmp_path = dest.with_extension("ipa.tmp");

    tx.send(Action::StatusMessage(format!(
        "Downloading {app_name} — see Downloads tab"
    )))
    .ok();

    if let Err(e) = stream_download(&client, &item.url, &tmp_path, &tx, id, &cancel_token).await {
        std::fs::remove_file(&tmp_path).ok();
        if cancel_token.is_cancelled() {
            tx.send(Action::DownloadCancelled(id)).ok();
        } else {
            tx.send(Action::DownloadError { id, error: e }).ok();
        }
        return;
    }

    tx.send(Action::DownloadProgress {
        id,
        stage: DownloadStage::Patching,
        progress: 0,
        total: 0,
    })
    .ok();

    if let Err(e) = ipatool_core::ipa::patch::patch_ipa(&tmp_path, &dest, &item, &account.email) {
        std::fs::remove_file(&tmp_path).ok();
        tx.send(Action::DownloadError {
            id,
            error: e.to_string(),
        })
        .ok();
        return;
    }

    std::fs::remove_file(&tmp_path).ok();
    tx.send(Action::DownloadComplete(id)).ok();
    tx.send(Action::StatusMessage(format!(
        "Download complete: {filename}"
    )))
    .ok();
}

async fn acquire_download_item(
    client: &Arc<Mutex<AppleClient>>,
    app_id: i64,
    account: &mut ipatool_core::model::Account,
    tx: &mpsc::UnboundedSender<Action>,
    id: usize,
) -> Result<ipatool_core::api::download::DownloadItem, String> {
    let max_attempts = 3;
    let mut purchased = false;

    for attempt in 0..max_attempts {
        let result = {
            let c = client.lock().await;
            ipatool_core::api::download::get_download_info(&c, app_id, account, None).await
        };

        match result {
            Ok(item) => return Ok(item),
            Err(e) if e.is_license_not_found() && !purchased => {
                tx.send(Action::DownloadProgress {
                    id,
                    stage: DownloadStage::Purchasing,
                    progress: 0,
                    total: 0,
                })
                .ok();
                purchase_for_download(client, app_id, account, tx).await?;
                purchased = true;
                tx.send(Action::StatusMessage(
                    "Purchase successful, fetching download info...".into(),
                ))
                .ok();
            }
            Err(e) if e.is_license_not_found() => {
                return Err("license not found after purchase".into());
            }
            Err(e) if e.is_token_expired() && attempt + 1 < max_attempts => {
                tx.send(Action::StatusMessage(
                    "Token expired, re-authenticating...".into(),
                ))
                .ok();
                let new_acc = {
                    let c = client.lock().await;
                    tui_reauth(&c, account)
                        .await
                        .map_err(|e| format!("re-auth failed: {e}"))?
                };
                *account = new_acc.clone();
                tx.send(Action::AccountRefreshed(new_acc.clone())).ok();
                client.lock().await.set_account(new_acc);
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    Err("failed to get download info after retries".into())
}

async fn purchase_for_download(
    client: &Arc<Mutex<AppleClient>>,
    app_id: i64,
    account: &mut ipatool_core::model::Account,
    tx: &mpsc::UnboundedSender<Action>,
) -> Result<(), String> {
    let result = {
        let c = client.lock().await;
        ipatool_core::api::purchase::purchase(&c, app_id, account).await
    };
    match result {
        Ok(()) => Ok(()),
        Err(e) if e.is_token_expired() => {
            tx.send(Action::StatusMessage(
                "Token expired, re-authenticating...".into(),
            ))
            .ok();
            let new_acc = {
                let c = client.lock().await;
                tui_reauth(&c, account)
                    .await
                    .map_err(|e| format!("re-auth failed: {e}"))?
            };
            *account = new_acc.clone();
            tx.send(Action::AccountRefreshed(new_acc)).ok();
            let mut c = client.lock().await;
            c.set_account(account.clone());
            match ipatool_core::api::purchase::purchase(&c, app_id, account).await {
                Ok(()) => Ok(()),
                Err(e) if e.is_license_already_exists() => Ok(()),
                Err(e) => Err(format!("purchase failed after re-auth: {e}")),
            }
        }
        Err(e) if e.is_license_already_exists() => Ok(()),
        Err(e) => Err(format!("purchase failed: {e}")),
    }
}

async fn stream_download(
    client: &Arc<Mutex<AppleClient>>,
    url: &str,
    tmp_path: &std::path::Path,
    tx: &mpsc::UnboundedSender<Action>,
    id: usize,
    cancel_token: &CancellationToken,
) -> Result<(), String> {
    let http = {
        let c = client.lock().await;
        c.http().clone()
    };

    let resp = http.get(url).send().await.map_err(|e| e.to_string())?;
    let total_size = resp.content_length().unwrap_or(0);

    tx.send(Action::DownloadProgress {
        id,
        stage: DownloadStage::Downloading,
        progress: 0,
        total: total_size,
    })
    .ok();

    let mut file = tokio::fs::File::create(tmp_path)
        .await
        .map_err(|e| e.to_string())?;

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_progress = tokio::time::Instant::now();
    let progress_interval = std::time::Duration::from_millis(100);

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        file.write_all(&bytes).await.map_err(|_| "write error".to_string())?;
                        downloaded += bytes.len() as u64;
                        let now = tokio::time::Instant::now();
                        if now.duration_since(last_progress) >= progress_interval {
                            last_progress = now;
                            tx.send(Action::DownloadProgress {
                                id,
                                stage: DownloadStage::Downloading,
                                progress: downloaded,
                                total: total_size,
                            })
                            .ok();
                        }
                    }
                    Some(Err(e)) => return Err(e.to_string()),
                    None => break,
                }
            }
            _ = cancel_token.cancelled() => {
                return Err("cancelled".into());
            }
        }
    }

    tx.send(Action::DownloadProgress {
        id,
        stage: DownloadStage::Downloading,
        progress: downloaded,
        total: total_size,
    })
    .ok();

    file.flush().await.map_err(|_| "flush error".to_string())?;
    Ok(())
}
