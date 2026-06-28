pub mod cookie_jar;
pub mod plist_xml;

use std::path::Path;
use std::sync::Arc;

use reqwest_cookie_store::CookieStoreMutex;

use crate::error::ClientError;
use crate::model::Account;

const USER_AGENT: &str =
    "Configurator/2.17 (Macintosh; OS X 15.2; 24C5089c) AppleWebKit/0620.1.16.11.6";

const MZFINANCE_AUTH_URL: &str =
    "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate";

pub struct AppleClient {
    http: reqwest::Client,
    cookie_store: Arc<CookieStoreMutex>,
    guid: String,
    account: Option<Account>,
}

impl AppleClient {
    pub fn new(guid: String, cookie_path: Option<&Path>) -> Result<Self, ClientError> {
        let cookie_store = cookie_jar::new_cookie_store(cookie_path)?;

        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .cookie_provider(cookie_store.clone())
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                let from_auth = attempt
                    .previous()
                    .last()
                    .is_some_and(|u| u.as_str() == MZFINANCE_AUTH_URL);
                if from_auth {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()?;

        Ok(Self {
            http,
            cookie_store,
            guid,
            account: None,
        })
    }

    pub fn guid(&self) -> &str {
        &self.guid
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    pub fn set_account(&mut self, account: Account) {
        self.account = Some(account);
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn save_cookies(&self, path: &Path) -> Result<(), ClientError> {
        cookie_jar::save_cookie_store(&self.cookie_store, path)
    }
}
