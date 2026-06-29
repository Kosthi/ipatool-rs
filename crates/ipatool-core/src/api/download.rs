use std::collections::HashMap;
use std::path::Path;

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;

#[derive(Debug, Deserialize)]
pub struct DownloadItem {
    #[serde(rename = "URL")]
    pub url: String,
    pub sinfs: Vec<Sinf>,
    #[serde(default)]
    pub metadata: HashMap<String, plist::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Sinf {
    pub id: i64,
    #[serde(with = "serde_bytes")]
    pub sinf: Vec<u8>,
}

mod serde_bytes {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let val = plist::Value::deserialize(deserializer)?;
        match val {
            plist::Value::Data(d) => Ok(d),
            _ => Err(serde::de::Error::custom("expected data")),
        }
    }
}

const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;

pub async fn get_download_info(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    external_version_id: Option<&str>,
) -> Result<DownloadItem, ClientError> {
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        match try_get_download_info(client, app_id, account, external_version_id).await {
            Err(ClientError::Store(StoreError::TemporarilyUnavailable))
                if attempt + 1 < MAX_DOWNLOAD_ATTEMPTS =>
            {
                let delay = 5 * (attempt as u64 + 1);
                tracing::warn!(
                    attempt = attempt + 1,
                    delay,
                    "temporarily unavailable, retrying"
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }
            other => return other,
        }
    }
    Err(ClientError::Store(StoreError::TemporarilyUnavailable))
}

async fn try_get_download_info(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    external_version_id: Option<&str>,
) -> Result<DownloadItem, ClientError> {
    let url = download_url(account, client.guid());

    let mut body = plist::Dictionary::new();
    body.insert(
        "salableAdamId".into(),
        plist::Value::String(app_id.to_string()),
    );
    body.insert(
        "guid".into(),
        plist::Value::String(client.guid().to_string()),
    );
    body.insert("creditDisplay".into(), plist::Value::String(String::new()));

    if let Some(vid) = external_version_id {
        body.insert(
            "externalVersionId".into(),
            plist::Value::String(vid.to_string()),
        );
    }

    let mut body_bytes = Vec::new();
    plist::to_writer_xml(&mut body_bytes, &body)
        .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

    tracing::debug!(
        %url,
        dsid = %account.directory_services_id,
        store_front = %account.store_front,
        token_len = account.password_token.len(),
        "download request"
    );

    let resp = client
        .http()
        .post(&url)
        .header("Content-Type", "application/x-apple-plist")
        .header("iCloud-DSID", &account.directory_services_id)
        .header("X-Dsid", &account.directory_services_id)
        .body(body_bytes)
        .send()
        .await?;

    let status = resp.status();
    tracing::debug!(%status, "download response status");

    let resp_body = resp.bytes().await?;
    tracing::debug!(
        len = resp_body.len(),
        preview = %String::from_utf8_lossy(&resp_body[..resp_body.len().min(500)]),
        "download response body"
    );
    let dict: HashMap<String, plist::Value> =
        crate::client::plist_xml::parse_plist_response(&resp_body)?;

    download_item_from_response_dict(&dict)
}

fn download_item_from_response_dict(
    dict: &HashMap<String, plist::Value>,
) -> Result<DownloadItem, ClientError> {
    match StoreError::from_plist_dict(dict) {
        Some(StoreError::LicenseAlreadyExists) => {
            if let Some(item) = download_item_from_dict(dict)? {
                Ok(item)
            } else {
                Err(ClientError::Store(StoreError::PasswordTokenExpired))
            }
        }
        Some(err) => Err(ClientError::Store(err)),
        None => {
            if let Some(item) = download_item_from_dict(dict)? {
                Ok(item)
            } else {
                Err(ClientError::UnexpectedResponse("missing songList".into()))
            }
        }
    }
}

fn download_item_from_dict(
    dict: &HashMap<String, plist::Value>,
) -> Result<Option<DownloadItem>, ClientError> {
    let Some(song_list) = dict.get("songList") else {
        return Ok(None);
    };

    let song_list = song_list
        .as_array()
        .ok_or_else(|| ClientError::UnexpectedResponse("invalid songList".into()))?;

    let first = song_list
        .first()
        .ok_or_else(|| ClientError::UnexpectedResponse("empty songList".into()))?;

    plist::from_value(first)
        .map(Some)
        .map_err(ClientError::PlistDe)
}

pub async fn download_file(
    client: &AppleClient,
    url: &str,
    dest: &Path,
    show_progress: bool,
) -> Result<(), ClientError> {
    let existing_size = if dest.exists() {
        tokio::fs::metadata(dest)
            .await
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };

    let mut req = client.http().get(url);
    if existing_size > 0 {
        req = req.header("Range", format!("bytes={existing_size}-"));
        tracing::info!(existing_size, "resuming download");
    }

    let resp = req.send().await?;

    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        tracing::info!("file already fully downloaded");
        return Ok(());
    }

    let total_size = resp.content_length().map(|cl| cl + existing_size);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dest)
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("open file: {e}")))?;

    let pb = if show_progress {
        let pb = match total_size {
            Some(total) => {
                let pb = ProgressBar::new(total);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{msg} [{bar:40}] {bytes}/{total_bytes} ({eta})")
                        .unwrap()
                        .progress_chars("=> "),
                );
                pb.set_position(existing_size);
                pb
            }
            None => {
                let pb = ProgressBar::new_spinner();
                pb.set_style(
                    ProgressStyle::default_spinner()
                        .template("{msg} {bytes} {elapsed}")
                        .unwrap(),
                );
                pb
            }
        };
        pb.set_message("Downloading");
        Some(pb)
    } else {
        None
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)
            .await
            .map_err(|e| ClientError::UnexpectedResponse(format!("write: {e}")))?;
        if let Some(ref pb) = pb {
            pb.inc(chunk.len() as u64);
        }
    }

    file.flush()
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("flush: {e}")))?;

    if let Some(pb) = pb {
        pb.finish_with_message("Download complete");
    }

    Ok(())
}

fn download_url(account: &Account, guid: &str) -> String {
    let host = match &account.pod {
        Some(pod) => format!("p{pod}-buy.itunes.apple.com"),
        None => "buy.itunes.apple.com".to_string(),
    };
    format!("https://{host}/WebObjects/MZFinance.woa/wa/volumeStoreDownloadProduct?guid={guid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_download_item() -> plist::Value {
        let mut sinf = plist::Dictionary::new();
        sinf.insert("id".into(), plist::Value::Integer(1.into()));
        sinf.insert("sinf".into(), plist::Value::Data(vec![1, 2, 3]));

        let mut item = plist::Dictionary::new();
        item.insert(
            "URL".into(),
            plist::Value::String("https://example.invalid/app.ipa".into()),
        );
        item.insert(
            "sinfs".into(),
            plist::Value::Array(vec![plist::Value::Dictionary(sinf)]),
        );

        plist::Value::Dictionary(item)
    }

    #[test]
    fn download_item_wins_over_license_already_exists_marker() {
        let mut dict = HashMap::new();
        dict.insert("failureType".into(), plist::Value::String("5002".into()));
        dict.insert(
            "customerMessage".into(),
            plist::Value::String("license already exists".into()),
        );
        dict.insert(
            "songList".into(),
            plist::Value::Array(vec![sample_download_item()]),
        );

        let item = download_item_from_response_dict(&dict).unwrap();

        assert_eq!(item.url, "https://example.invalid/app.ipa");
        assert_eq!(item.sinfs[0].id, 1);
        assert_eq!(item.sinfs[0].sinf, vec![1, 2, 3]);
    }

    #[test]
    fn bare_license_already_exists_requires_reauth_for_download() {
        let mut dict = HashMap::new();
        dict.insert("failureType".into(), plist::Value::String("5002".into()));
        dict.insert(
            "customerMessage".into(),
            plist::Value::String("license already exists".into()),
        );

        let err = download_item_from_response_dict(&dict).unwrap_err();

        assert!(err.is_token_expired());
    }
}
