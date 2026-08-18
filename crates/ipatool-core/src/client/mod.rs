pub mod cookie_jar;
pub mod plist_xml;

use std::path::Path;
use std::sync::Arc;

use reqwest_cookie_store::CookieStoreMutex;

use crate::error::ClientError;
use crate::model::Account;

const USER_AGENT: &str =
    "Configurator/2.17 (Macintosh; OS X 15.2; 24C5089c) AppleWebKit/0620.1.16.11.6";

/// Auth endpoints 302-redirect POSTs to account-specific pod hosts
/// (e.g. p14-buy.itunes.apple.com); auto-following would turn the POST into a
/// bodyless GET, so those redirects must surface to api::auth::login, which
/// re-sends the full request. Matched by path so it covers the bare host, pod
/// hosts, and the native /fast/ endpoint, with or without a trailing slash.
fn is_auth_endpoint(url: &reqwest::Url) -> bool {
    let path = url.path().trim_end_matches('/');
    path.ends_with("/wa/authenticate") || path.ends_with("/native/fast")
}

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
                let from_auth = attempt.previous().last().is_some_and(is_auth_endpoint);
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

#[cfg(test)]
mod tests {
    use super::is_auth_endpoint;

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn auth_endpoint_matches_with_and_without_trailing_slash() {
        assert!(is_auth_endpoint(&url(
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
        )));
        assert!(is_auth_endpoint(&url(
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
        )));
    }

    #[test]
    fn auth_endpoint_matches_pod_hosts_and_native_fast() {
        assert!(is_auth_endpoint(&url(
            "https://p14-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
        )));
        assert!(is_auth_endpoint(&url(
            "https://auth.itunes.apple.com/auth/v1/native/fast/"
        )));
    }

    #[test]
    fn non_auth_endpoints_do_not_match() {
        assert!(!is_auth_endpoint(&url(
            "https://p14-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/volumeStoreDownloadProduct?guid=X"
        )));
        assert!(!is_auth_endpoint(&url(
            "https://init.itunes.apple.com/bag.xml?ix=5"
        )));
    }
}
