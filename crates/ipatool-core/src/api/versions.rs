use std::collections::HashMap;

use serde::Serialize;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;

#[derive(Debug, Serialize)]
pub struct ListVersionsOutput {
    pub app_id: i64,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Serialize)]
pub struct VersionEntry {
    pub external_version_id: String,
    pub version_string: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VersionMetadata {
    pub bundle_short_version: Option<String>,
    pub bundle_version: Option<String>,
    pub release_date: Option<String>,
    pub extra: HashMap<String, plist::Value>,
}

pub async fn list_versions(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
) -> Result<ListVersionsOutput, ClientError> {
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

    let mut body_bytes = Vec::new();
    plist::to_writer_xml(&mut body_bytes, &body)
        .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

    let resp = client
        .http()
        .post(&url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("iCloud-DSID", &account.directory_services_id)
        .header("X-Dsid", &account.directory_services_id)
        .header("X-Apple-Store-Front", &account.store_front)
        .header("X-Token", &account.password_token)
        .body(body_bytes)
        .send()
        .await?;

    let resp_body = resp.bytes().await?;
    let dict: HashMap<String, plist::Value> =
        crate::client::plist_xml::parse_plist_response(&resp_body)?;

    if let Some(err) = StoreError::from_plist_dict(&dict) {
        return Err(ClientError::Store(err));
    }

    let mut versions = Vec::new();

    if let Some(plist::Value::Dictionary(song_list_map)) = dict.get("songList") {
        for (ext_id, _value) in song_list_map {
            versions.push(VersionEntry {
                external_version_id: ext_id.clone(),
                version_string: None,
            });
        }
    }

    if let Some(plist::Value::Array(items)) = dict.get("songList")
        && let Some(first) = items.first()
        && let Some(avail) = first
            .as_dictionary()
            .and_then(|d| d.get("externalVersionIdentifiers"))
            .and_then(|v| v.as_array())
    {
        versions.clear();
        for v in avail {
            let id_str = match v {
                plist::Value::Integer(i) => {
                    i.as_signed().map(|n| n.to_string()).unwrap_or_default()
                }
                plist::Value::String(s) => s.clone(),
                _ => continue,
            };
            versions.push(VersionEntry {
                external_version_id: id_str,
                version_string: None,
            });
        }
    }

    Ok(ListVersionsOutput { app_id, versions })
}

pub async fn get_version_metadata(
    client: &AppleClient,
    app_id: i64,
    account: &Account,
    external_version_id: &str,
) -> Result<VersionMetadata, ClientError> {
    let item =
        crate::api::download::get_download_info(client, app_id, account, Some(external_version_id))
            .await?;

    let bundle_short_version = item
        .metadata
        .get("bundleShortVersionString")
        .and_then(|v| v.as_string())
        .map(String::from);

    let bundle_version = item
        .metadata
        .get("bundleVersion")
        .and_then(|v| v.as_string())
        .map(String::from);

    let release_date = item
        .metadata
        .get("releaseDate")
        .and_then(|v| v.as_string())
        .map(String::from);

    Ok(VersionMetadata {
        bundle_short_version,
        bundle_version,
        release_date,
        extra: item.metadata,
    })
}

fn download_url(account: &Account, guid: &str) -> String {
    let host = match &account.pod {
        Some(pod) => format!("p{pod}-buy.itunes.apple.com"),
        None => "buy.itunes.apple.com".to_string(),
    };
    format!("https://{host}/WebObjects/MZFinance.woa/wa/volumeStoreDownloadProduct?guid={guid}")
}
