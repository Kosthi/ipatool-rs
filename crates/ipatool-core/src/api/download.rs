use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
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
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

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
    tracing::debug!(len = resp_body.len(), "download response body received");
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
    download_file_with_connections(client, url, dest, show_progress, 1).await
}

pub async fn download_file_with_connections(
    client: &AppleClient,
    url: &str,
    dest: &Path,
    show_progress: bool,
    connections: usize,
) -> Result<(), ClientError> {
    if connections <= 1 {
        return download_file_sequential(client, url, dest, show_progress).await;
    }

    match fetch_download_size(client, url).await {
        Ok(size) => {
            download_file_parallel(client, url, dest, show_progress, connections, size).await
        }
        Err(err) => {
            tracing::warn!(%err, "parallel download unavailable, falling back to sequential");
            download_file_sequential(client, url, dest, show_progress).await
        }
    }
}

async fn download_file_sequential(
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

    let resp = req.header("Accept-Encoding", "identity").send().await?;

    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        tracing::info!("file already fully downloaded");
        return Ok(());
    }

    let status = resp.status();
    let resumed = existing_size > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    let restart = existing_size > 0 && status == reqwest::StatusCode::OK;
    if existing_size > 0 && restart {
        tracing::warn!("server ignored range request, restarting download");
    } else if existing_size > 0 && !resumed {
        return Err(unexpected_download_response(&resp, "resume download"));
    } else if existing_size == 0 && !status.is_success() {
        return Err(unexpected_download_response(&resp, "download"));
    }
    if looks_like_error_body(&resp) {
        return Err(unexpected_download_response(&resp, "download"));
    }

    let starting_position = if resumed { existing_size } else { 0 };
    let total_size = resp.content_length().map(|cl| cl + starting_position);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(resumed)
        .write(true)
        .truncate(!resumed)
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
                pb.set_position(starting_position);
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
    while let Some(chunk) = next_download_chunk(&mut stream).await? {
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

async fn download_file_parallel(
    client: &AppleClient,
    url: &str,
    dest: &Path,
    show_progress: bool,
    connections: usize,
    total_size: u64,
) -> Result<(), ClientError> {
    let connections = connections.clamp(1, 16);
    let part_size = total_size.div_ceil(connections as u64);
    let part_paths: Vec<PathBuf> = (0..connections)
        .map(|idx| dest.with_extension(format!("ipa.part{idx}")))
        .collect();

    tokio::fs::remove_file(dest).await.ok();
    for path in &part_paths {
        tokio::fs::remove_file(path).await.ok();
    }

    let pb = if show_progress {
        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg} [{bar:40}] {bytes}/{total_bytes} {binary_bytes_per_sec} ({eta})")
                .unwrap()
                .progress_chars("=> "),
        );
        pb.set_message(format!("Downloading ({connections} connections)"));
        Some(pb)
    } else {
        None
    };

    let mut tasks = FuturesUnordered::new();
    for (idx, path) in part_paths.iter().enumerate().take(connections) {
        let start = idx as u64 * part_size;
        if start >= total_size {
            continue;
        }

        let end = (start + part_size - 1).min(total_size - 1);
        let path = path.clone();
        let http = client.http().clone();
        let url = url.to_string();
        let pb = pb.clone();
        tasks.push(tokio::spawn(async move {
            download_range(http, url, path, start, end, pb).await
        }));
    }

    while let Some(result) = tasks.next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                cleanup_parts(&part_paths).await;
                if let Some(pb) = pb {
                    pb.abandon_with_message("Download failed");
                }
                return Err(err);
            }
            Err(err) => {
                cleanup_parts(&part_paths).await;
                if let Some(pb) = pb {
                    pb.abandon_with_message("Download failed");
                }
                return Err(ClientError::UnexpectedResponse(format!(
                    "download task failed: {err}"
                )));
            }
        }
    }

    let mut dest_file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("create destination: {e}")))?;
    for path in &part_paths {
        let mut part = tokio::fs::File::open(path)
            .await
            .map_err(|e| ClientError::UnexpectedResponse(format!("open part: {e}")))?;
        tokio::io::copy(&mut part, &mut dest_file)
            .await
            .map_err(|e| ClientError::UnexpectedResponse(format!("copy part: {e}")))?;
    }
    dest_file
        .flush()
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("flush destination: {e}")))?;
    cleanup_parts(&part_paths).await;

    if let Some(pb) = pb {
        pb.finish_with_message("Download complete");
    }

    Ok(())
}

async fn download_range(
    http: reqwest::Client,
    url: String,
    path: PathBuf,
    start: u64,
    end: u64,
    pb: Option<ProgressBar>,
) -> Result<(), ClientError> {
    let resp = http
        .get(&url)
        .header("Accept-Encoding", "identity")
        .header("Range", format!("bytes={start}-{end}"))
        .send()
        .await?;

    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT || looks_like_error_body(&resp) {
        return Err(unexpected_download_response(&resp, "range download"));
    }

    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("create part: {e}")))?;
    let mut stream = resp.bytes_stream();
    let mut downloaded = 0u64;
    let expected = end - start + 1;

    while let Some(chunk) = next_download_chunk(&mut stream).await? {
        file.write_all(&chunk)
            .await
            .map_err(|e| ClientError::UnexpectedResponse(format!("write part: {e}")))?;
        downloaded += chunk.len() as u64;
        if let Some(ref pb) = pb {
            pb.inc(chunk.len() as u64);
        }
    }

    file.flush()
        .await
        .map_err(|e| ClientError::UnexpectedResponse(format!("flush part: {e}")))?;

    if downloaded != expected {
        return Err(ClientError::UnexpectedResponse(format!(
            "short range download: expected {expected} bytes, got {downloaded}"
        )));
    }

    Ok(())
}

async fn fetch_download_size(client: &AppleClient, url: &str) -> Result<u64, ClientError> {
    let resp = client
        .http()
        .get(url)
        .header("Accept-Encoding", "identity")
        .header("Range", "bytes=0-0")
        .send()
        .await?;

    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT || looks_like_error_body(&resp) {
        return Err(unexpected_download_response(&resp, "download size probe"));
    }

    resp.headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.rsplit('/').next())
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| ClientError::UnexpectedResponse("missing Content-Range total".into()))
}

async fn next_download_chunk(
    stream: &mut (impl StreamExt<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin),
) -> Result<Option<bytes::Bytes>, ClientError> {
    tokio::time::timeout(DOWNLOAD_IDLE_TIMEOUT, stream.next())
        .await
        .map_err(|_| {
            ClientError::UnexpectedResponse(format!(
                "download stalled for {} seconds",
                DOWNLOAD_IDLE_TIMEOUT.as_secs()
            ))
        })?
        .transpose()
        .map_err(ClientError::Http)
}

fn unexpected_download_response(resp: &reqwest::Response, context: &str) -> ClientError {
    ClientError::UnexpectedResponse(format!(
        "{context}: HTTP {}, content-type {}, content-length {}",
        resp.status(),
        resp.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<missing>"),
        resp.content_length()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "<missing>".into())
    ))
}

fn looks_like_error_body(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|content_type| {
            let content_type = content_type.to_ascii_lowercase();
            content_type.starts_with("text/")
                || content_type.contains("html")
                || content_type.contains("json")
        })
        .unwrap_or(false)
}

async fn cleanup_parts(paths: &[PathBuf]) {
    for path in paths {
        tokio::fs::remove_file(path).await.ok();
    }
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
