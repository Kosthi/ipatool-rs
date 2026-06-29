use std::collections::HashMap;
use std::time::Duration;

use url::Url;

use crate::client::AppleClient;
use crate::error::ClientError;

const BAG_URL: &str = "https://init.itunes.apple.com/bag.xml?ix=5";
const BAG_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn fetch_auth_endpoint(client: &AppleClient) -> Result<Url, ClientError> {
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
        .and_then(|v| v.as_data())
        .ok_or_else(|| ClientError::UnexpectedResponse("bag: missing 'bag' data".into()))?;

    let inner: HashMap<String, plist::Value> =
        plist::from_bytes(bag_data).map_err(ClientError::PlistDe)?;

    let url_str = inner
        .get("authenticateAccount")
        .and_then(|v| v.as_string())
        .ok_or_else(|| {
            ClientError::UnexpectedResponse("bag: missing authenticateAccount URL".into())
        })?;

    let mut url = Url::parse(url_str)
        .map_err(|e| ClientError::UnexpectedResponse(format!("bag: invalid URL: {e}")))?;

    // Apple's native auth endpoint needs /fast/ suffix with trailing slash
    // to avoid a redirect that strips the POST body
    let path = url.path().to_string();
    if path.contains("/native") && !path.contains("/fast") {
        url.set_path(&format!("{}/fast/", path.trim_end_matches('/')));
    } else if !path.ends_with('/') {
        url.set_path(&format!("{}/", path));
    }

    Ok(url)
}
