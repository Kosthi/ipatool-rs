use std::sync::Arc;

use ratatui::widgets::{ListState, TableState};
use tokio::sync::{Mutex, Semaphore};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use tui_input::Input;

use ipatool_core::client::AppleClient;
use ipatool_core::model::storefront::country_code_from_store_front;
use ipatool_core::model::{Account, App, Platform};

use super::action::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTab {
    Search,
    Library,
    Downloads,
    Account,
}

impl ActiveTab {
    pub const ALL: [ActiveTab; 4] = [
        ActiveTab::Search,
        ActiveTab::Library,
        ActiveTab::Downloads,
        ActiveTab::Account,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Self::Search => "Search",
            Self::Library => "Library",
            Self::Downloads => "Downloads",
            Self::Account => "Account",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    SearchInput,
    LoginEmail,
    LoginPassword,
    LoginAuthCode,
    Popup(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadStage {
    Queued,
    Purchasing,
    Downloading,
    Patching,
    Complete,
    Failed,
    Cancelled,
}

impl std::fmt::Display for DownloadStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => write!(f, "Queued"),
            Self::Purchasing => write!(f, "Purchasing"),
            Self::Downloading => write!(f, "Downloading"),
            Self::Patching => write!(f, "Patching"),
            Self::Complete => write!(f, "Complete"),
            Self::Failed => write!(f, "Failed"),
            Self::Cancelled => write!(f, "Cancelled"),
        }
    }
}

pub struct DownloadTask {
    pub id: usize,
    pub app_name: String,
    pub stage: DownloadStage,
    pub progress: u64,
    pub total: u64,
    pub error: Option<String>,
    pub cancel_token: CancellationToken,
}

pub struct App_ {
    pub active_tab: ActiveTab,
    pub input_mode: InputMode,
    pub should_quit: bool,
    pub action_tx: UnboundedSender<Action>,

    pub search_input: Input,
    pub search_results: Vec<App>,
    pub search_table_state: TableState,
    pub search_platform: Platform,
    pub search_country: String,
    pub is_loading: bool,
    pub selected_detail: Option<App>,

    pub downloads: Vec<DownloadTask>,
    pub download_list_state: ListState,
    pub download_semaphore: Arc<Semaphore>,
    pub next_download_id: usize,

    pub account: Option<Account>,
    pub login_email: Input,
    pub login_password: String,
    pub login_auth_code: Input,
    pub login_needs_auth_code: bool,
    pub login_error: Option<String>,

    pub client: Arc<Mutex<AppleClient>>,

    pub status_message: String,
}

impl App_ {
    pub fn new(client: AppleClient, action_tx: UnboundedSender<Action>) -> Self {
        let account = ipatool_core::credential::load_account().ok().flatten();

        let search_country = account
            .as_ref()
            .and_then(|a| country_code_from_store_front(&a.store_front))
            .unwrap_or("US")
            .to_string();

        let client = {
            let mut c = client;
            if let Some(ref acc) = account {
                c.set_account(acc.clone());
            }
            Arc::new(Mutex::new(c))
        };

        Self {
            active_tab: ActiveTab::Search,
            input_mode: InputMode::Normal,
            should_quit: false,
            action_tx,

            search_input: Input::default(),
            search_results: Vec::new(),
            search_table_state: TableState::default(),
            search_platform: Platform::IPhone,
            search_country,
            is_loading: false,
            selected_detail: None,

            downloads: Vec::new(),
            download_list_state: ListState::default(),
            download_semaphore: Arc::new(Semaphore::new(3)),
            next_download_id: 0,

            account,
            login_email: Input::default(),
            login_password: String::new(),
            login_auth_code: Input::default(),
            login_needs_auth_code: false,
            login_error: None,

            client,

            status_message: String::new(),
        }
    }

    pub fn selected_app(&self) -> Option<&App> {
        let idx = self.search_table_state.selected()?;
        self.search_results.get(idx)
    }

    pub fn update_selected_detail(&mut self) {
        self.selected_detail = self.selected_app().cloned();
    }

    pub fn download_by_id(&self, id: usize) -> Option<&DownloadTask> {
        self.downloads.iter().find(|d| d.id == id)
    }

    pub fn download_by_id_mut(&mut self, id: usize) -> Option<&mut DownloadTask> {
        self.downloads.iter_mut().find(|d| d.id == id)
    }

    pub fn clear_finished_downloads(&mut self) {
        self.downloads.retain(|d| {
            !matches!(
                d.stage,
                DownloadStage::Complete | DownloadStage::Failed | DownloadStage::Cancelled
            )
        });
        if self.downloads.is_empty() {
            self.download_list_state.select(None);
        } else if let Some(sel) = self.download_list_state.selected() {
            if sel >= self.downloads.len() {
                self.download_list_state
                    .select(Some(self.downloads.len() - 1));
            }
        }
    }
}
