use ipatool_core::model::{Account, App};

use super::app::{ActiveTab, DownloadStage};

#[derive(Debug)]
#[allow(dead_code)]
pub enum Action {
    Quit,
    Tick,
    Render,

    SwitchTab(ActiveTab),

    SubmitSearch,
    SearchResults(Vec<App>),
    SearchError(String),

    StartDownload {
        bundle_id: String,
        app_name: String,
        app_id: i64,
    },
    DownloadProgress {
        id: usize,
        stage: DownloadStage,
        progress: u64,
        total: u64,
    },
    DownloadComplete(usize),
    DownloadError {
        id: usize,
        error: String,
    },
    CancelDownload(usize),
    DownloadCancelled(usize),
    ClearFinishedDownloads,

    SubmitLogin,
    LoginSuccess(Account),
    LoginError(String),
    Logout,
    AccountRefreshed(Account),

    Purchase(i64, String),
    PurchaseSuccess(String),
    PurchaseError(String),

    ShowPopup(String),
    ClosePopup,
    StatusMessage(String),
}
