use crate::client::AppleClient;
use crate::error::ClientError;
use crate::model::app::{App, SearchResponse};
use crate::model::platform::Platform;

const SEARCH_URL: &str = "https://itunes.apple.com/search";

pub async fn search(
    client: &AppleClient,
    term: &str,
    country: &str,
    platform: Platform,
    limit: u32,
) -> Result<Vec<App>, ClientError> {
    let resp = client
        .http()
        .get(SEARCH_URL)
        .query(&[
            ("term", term),
            ("country", country),
            ("entity", platform.search_entity()),
            ("limit", &limit.to_string()),
        ])
        .send()
        .await?;

    let body = resp.text().await?;
    let search_resp: SearchResponse =
        serde_json::from_str(&body).map_err(|e| ClientError::UnexpectedResponse(e.to_string()))?;

    Ok(search_resp.results)
}
