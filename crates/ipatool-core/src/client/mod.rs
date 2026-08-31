pub mod cookie_jar;
pub mod plist_xml;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use reqwest_cookie_store::CookieStoreMutex;

use crate::error::ClientError;
use crate::guid::MachineIdentity;
use crate::model::Account;

/// Identifies this client to Apple as Apple Configurator.
///
/// This is not cosmetic: `init.itunes.apple.com/bag.xml` hands out a different
/// `authenticateAccount` depending on who is asking, and the whole sign-in path
/// — the endpoint allowlist in [`crate::api::bag`] and the SAP handshake it
/// configures — is built around what the Configurator UA gets back.
///
/// | User-Agent          | `authenticateAccount`                        | `sign-sap-setup`               |
/// |---------------------|----------------------------------------------|--------------------------------|
/// | `Configurator/2.17` | `buy.itunes.apple.com/…/wa/authenticate`     | `…/signSapSetup/legacy`        |
/// | `iTunes/12.12`      | `auth.itunes.apple.com/auth/v1/native`       | `…/signSapSetup` (no `legacy`) |
/// | none                | `auth.itunes.apple.com/auth/v1/native/fast`  | `…/signSapSetup/legacy`        |
///
/// So changing this changes which endpoint the bag routes to, and can change
/// the SAP setup URL along with it — possibly a different handshake variant.
/// See issue #17, where the endpoint that turned up in a bag dump was mistaken
/// for a client-independent migration.
const USER_AGENT: &str =
    "Configurator/2.17 (Macintosh; OS X 15.2; 24C5089c) AppleWebKit/0620.1.16.11.6";

/// Auth endpoints 302-redirect POSTs to account-specific pod hosts
/// (e.g. p14-buy.itunes.apple.com); auto-following would turn the POST into a
/// bodyless GET, so those redirects must surface to api::auth::login, which
/// re-sends the full request. Matched by path so it covers the bare host and
/// pod hosts alike, with or without a trailing slash.
fn is_auth_endpoint(url: &reqwest::Url) -> bool {
    url.path()
        .trim_end_matches('/')
        .ends_with("/wa/authenticate")
}

pub struct AppleClient {
    http: reqwest::Client,
    cookie_store: Arc<CookieStoreMutex>,
    machine: MachineIdentity,
    /// Where the SAP runtime keeps the Apple frameworks and the emulator it
    /// downloads; they are large and pinned by digest, so they are fetched once.
    cache_dir: PathBuf,
    account: Option<Account>,
}

impl AppleClient {
    pub fn new(
        machine: MachineIdentity,
        cookie_path: Option<&Path>,
        cache_dir: impl Into<PathBuf>,
    ) -> Result<Self, ClientError> {
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
            machine,
            cache_dir: cache_dir.into(),
            account: None,
        })
    }

    pub fn guid(&self) -> &str {
        &self.machine.guid
    }

    /// The bytes Apple's SAP handshake binds its session key to.
    pub fn hardware_id(&self) -> &[u8] {
        &self.machine.hardware_id
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
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

    /// A client with a fixed identity and a shared scratch cache.
    ///
    /// The cache is deliberately shared between tests: the live ones download
    /// tens of megabytes, and re-fetching per test would be slow and unkind to
    /// the servers involved.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self::new(
            crate::guid::MachineIdentity {
                guid: "AABBCCDDEEFF".into(),
                hardware_id: vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF],
            },
            None,
            std::env::temp_dir().join("ipatool-rs-test-cache"),
        )
        .expect("build test client")
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
    fn auth_endpoint_matches_pod_hosts() {
        assert!(is_auth_endpoint(&url(
            "https://p14-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
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
