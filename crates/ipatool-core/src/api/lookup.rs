use crate::client::AppleClient;
use crate::error::ClientError;
use crate::model::app::{App, SearchResponse};
use crate::model::platform::Platform;

const LOOKUP_URL: &str = "https://itunes.apple.com/lookup";

pub async fn lookup(
    client: &AppleClient,
    bundle_id: &str,
    country: &str,
    platform: Platform,
) -> Result<Option<App>, ClientError> {
    let resp = client
        .http()
        .get(LOOKUP_URL)
        .query(&[
            ("bundleId", bundle_id),
            ("country", country),
            ("entity", platform.lookup_entity()),
            ("limit", "1"),
        ])
        .send()
        .await?;

    let body = resp.text().await?;
    let search_resp: SearchResponse =
        serde_json::from_str(&body).map_err(|e| ClientError::UnexpectedResponse(e.to_string()))?;

    Ok(search_resp.results.into_iter().next())
}
