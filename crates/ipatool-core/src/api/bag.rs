use std::collections::HashMap;
use std::time::Duration;

use url::Url;

use crate::client::AppleClient;
use crate::error::ClientError;
use crate::sap::SapConfig;

const BAG_URL: &str = "https://init.itunes.apple.com/bag.xml?ix=5";
const BAG_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The only sign-in action Apple's bag advertises.
pub const AUTH_PATH: &str = "/WebObjects/MZFinance.woa/wa/authenticate";

const AUTH_HOST: &str = "buy.itunes.apple.com";
/// Account-specific pods, e.g. `p14-buy.itunes.apple.com`.
const AUTH_POD_SUFFIX: &str = "-buy.itunes.apple.com";

#[derive(Debug, Clone)]
pub struct BagConfig {
    pub auth_endpoint: Url,
    pub sap: SapConfig,
}

pub async fn fetch_bag(client: &AppleClient) -> Result<BagConfig, ClientError> {
    let resp = client
        .http()
        .get(BAG_URL)
        .timeout(BAG_REQUEST_TIMEOUT)
        .send()
        .await?;
    let body = resp.bytes().await?;

    let outer: HashMap<String, plist::Value> =
        crate::client::plist_xml::parse_plist_response(&body)?;

    let bag_data = outer
        .get("bag")
        .and_then(plist::Value::as_data)
        .ok_or_else(|| ClientError::UnexpectedResponse("bag: missing 'bag' data".into()))?;

    let inner: HashMap<String, plist::Value> =
        plist::from_bytes(bag_data).map_err(ClientError::PlistDe)?;

    parse_bag(&inner)
}

fn parse_bag(inner: &HashMap<String, plist::Value>) -> Result<BagConfig, ClientError> {
    let auth_endpoint = bag_url(inner, "authenticateAccount")?;
    validate_auth_endpoint(&auth_endpoint)?;

    // Apple publishes the version as a string.
    let version_str = inner
        .get("sign-sap-version")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| ClientError::UnexpectedResponse("bag: missing sign-sap-version".into()))?;

    let version = version_str.parse::<u32>().map_err(|e| {
        ClientError::UnexpectedResponse(format!(
            "bag: invalid sign-sap-version {version_str:?}: {e}"
        ))
    })?;

    let sap = SapConfig {
        setup_url: bag_url(inner, "sign-sap-setup")?,
        cert_url: bag_url(inner, "sign-sap-setup-cert")?,
        version,
    };
    sap.validate()?;

    Ok(BagConfig { auth_endpoint, sap })
}

fn bag_url(inner: &HashMap<String, plist::Value>, key: &str) -> Result<Url, ClientError> {
    let raw = inner
        .get(key)
        .and_then(plist::Value::as_string)
        .ok_or_else(|| ClientError::UnexpectedResponse(format!("bag: missing {key}")))?;

    Url::parse(raw)
        .map_err(|e| ClientError::UnexpectedResponse(format!("bag: invalid {key} URL: {e}")))
}

/// Credentials are POSTed to this URL, and Apple redirects sign-in to
/// account-specific pods, so every candidate — from the bag and from a redirect
/// alike — is checked against the shape Apple actually uses.
pub fn validate_auth_endpoint(url: &Url) -> Result<(), ClientError> {
    if url.scheme() != "https" {
        return Err(ClientError::UnexpectedResponse(format!(
            "unsupported authentication endpoint {url}: not HTTPS"
        )));
    }

    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host != AUTH_HOST && !host.ends_with(AUTH_POD_SUFFIX) {
        return Err(ClientError::UnexpectedResponse(format!(
            "unsupported authentication endpoint {url}: unexpected host"
        )));
    }

    if url.path().trim_end_matches('/') != AUTH_PATH {
        return Err(ClientError::UnexpectedResponse(format!(
            "unsupported authentication endpoint {url}: unexpected path"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// Mirrors what init.itunes.apple.com returns today.
    fn current_bag() -> HashMap<String, plist::Value> {
        HashMap::from([
            (
                "authenticateAccount".to_string(),
                plist::Value::String(
                    "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate".into(),
                ),
            ),
            (
                "sign-sap-setup".to_string(),
                plist::Value::String(
                    "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy".into(),
                ),
            ),
            (
                "sign-sap-setup-cert".to_string(),
                plist::Value::String("https://s.mzstatic.com/sap/setupCert.plist".into()),
            ),
            (
                "sign-sap-version".to_string(),
                plist::Value::String("200".into()),
            ),
        ])
    }

    #[test]
    fn parses_apples_current_bag() {
        let config = parse_bag(&current_bag()).unwrap();

        assert_eq!(
            config.auth_endpoint.as_str(),
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
        );
        assert_eq!(
            config.sap.setup_url.as_str(),
            "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy"
        );
        assert_eq!(config.sap.version, 200);
    }

    #[test]
    fn fails_when_sap_keys_are_absent() {
        let mut bag = current_bag();
        bag.remove("sign-sap-setup");

        assert!(parse_bag(&bag).is_err());
    }

    #[test]
    fn accepts_pod_hosts() {
        assert!(
            validate_auth_endpoint(&url(
                "https://p14-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
            ))
            .is_ok()
        );
    }

    #[test]
    fn accepts_trailing_slash() {
        assert!(
            validate_auth_endpoint(&url(
                "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
            ))
            .is_ok()
        );
    }

    /// The endpoint the previous fix targeted. Apple lists it under
    /// `sign-sap-request` too, so it needs a signature just the same and is no
    /// longer a fallback worth having.
    #[test]
    fn rejects_native_fast_endpoint() {
        assert!(
            validate_auth_endpoint(&url("https://auth.itunes.apple.com/auth/v1/native/fast/"))
                .is_err()
        );
    }

    #[test]
    fn rejects_lookalike_host() {
        assert!(
            validate_auth_endpoint(&url(
                "https://buy.itunes.apple.com.evil.test/WebObjects/MZFinance.woa/wa/authenticate"
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_other_action_on_valid_host() {
        assert!(
            validate_auth_endpoint(&url(
                "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/buyProduct"
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_plaintext() {
        assert!(
            validate_auth_endpoint(&url(
                "http://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
            ))
            .is_err()
        );
    }
}
