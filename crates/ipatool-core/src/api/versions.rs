use std::collections::HashMap;

use serde::Serialize;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;

#[derive(Debug, Serialize)]
pub struct ListVersionsOutput {
    pub app_id: i64,
    pub versions: Vec<VersionEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_external_version_id: Option<String>,
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
    body.insert("creditDisplay".into(), plist::Value::String(String::new()));

    let mut body_bytes = Vec::new();
    plist::to_writer_xml(&mut body_bytes, &body)
        .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

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
    tracing::debug!(%status, "list versions response status");

    let resp_body = resp.bytes().await?;
    let dict: HashMap<String, plist::Value> =
        crate::client::plist_xml::parse_plist_response(&resp_body)?;

    list_versions_from_response_dict(app_id, &dict)
}

fn list_versions_from_response_dict(
    app_id: i64,
    dict: &HashMap<String, plist::Value>,
) -> Result<ListVersionsOutput, ClientError> {
    match StoreError::from_plist_dict(dict) {
        Some(StoreError::LicenseAlreadyExists) if dict.contains_key("songList") => {}
        Some(err) => return Err(ClientError::Store(err)),
        None => {}
    }

    if let Some(plist::Value::Dictionary(song_list_map)) = dict.get("songList") {
        let mut versions = Vec::new();
        for (ext_id, _value) in song_list_map {
            versions.push(VersionEntry {
                external_version_id: ext_id.clone(),
                version_string: None,
            });
        }
        return Ok(ListVersionsOutput {
            app_id,
            versions,
            latest_external_version_id: None,
        });
    }

    let song_list = dict
        .get("songList")
        .ok_or_else(|| ClientError::UnexpectedResponse("missing songList".into()))?
        .as_array()
        .ok_or_else(|| ClientError::UnexpectedResponse("invalid songList".into()))?;

    let first = song_list
        .first()
        .ok_or_else(|| ClientError::UnexpectedResponse("empty songList".into()))?;

    let item = first
        .as_dictionary()
        .ok_or_else(|| ClientError::UnexpectedResponse("invalid songList item".into()))?;

    let (versions, latest_external_version_id) = versions_from_item(item)?;

    Ok(ListVersionsOutput {
        app_id,
        versions,
        latest_external_version_id,
    })
}

fn versions_from_item(
    item: &plist::Dictionary,
) -> Result<(Vec<VersionEntry>, Option<String>), ClientError> {
    if let Some(metadata) = item.get("metadata").and_then(|v| v.as_dictionary()) {
        return versions_from_metadata(
            metadata,
            "softwareVersionExternalIdentifiers",
            "softwareVersionExternalIdentifier",
        );
    }

    versions_from_metadata(
        item,
        "externalVersionIdentifiers",
        "externalVersionIdentifier",
    )
}

fn versions_from_metadata(
    metadata: &plist::Dictionary,
    ids_key: &str,
    latest_key: &str,
) -> Result<(Vec<VersionEntry>, Option<String>), ClientError> {
    let version_ids = metadata
        .get(ids_key)
        .ok_or_else(|| ClientError::UnexpectedResponse(format!("missing {ids_key}")))?
        .as_array()
        .ok_or_else(|| ClientError::UnexpectedResponse(format!("invalid {ids_key}")))?;

    let versions = version_ids
        .iter()
        .map(|v| {
            Ok(VersionEntry {
                external_version_id: version_id_value_to_string(v)?,
                version_string: None,
            })
        })
        .collect::<Result<Vec<_>, ClientError>>()?;

    let latest_external_version_id = metadata
        .get(latest_key)
        .map(version_id_value_to_string)
        .transpose()?;

    Ok((versions, latest_external_version_id))
}

fn version_id_value_to_string(value: &plist::Value) -> Result<String, ClientError> {
    match value {
        plist::Value::Integer(i) => i
            .as_signed()
            .map(|n| n.to_string())
            .ok_or_else(|| ClientError::UnexpectedResponse("invalid version identifier".into())),
        plist::Value::String(s) => Ok(s.clone()),
        _ => Err(ClientError::UnexpectedResponse(
            "invalid version identifier".into(),
        )),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn version_item(metadata: plist::Dictionary) -> plist::Value {
        let mut item = plist::Dictionary::new();
        item.insert("metadata".into(), plist::Value::Dictionary(metadata));
        plist::Value::Dictionary(item)
    }

    fn response_with_song_list(song_list: plist::Value) -> HashMap<String, plist::Value> {
        let mut dict = HashMap::new();
        dict.insert("songList".into(), song_list);
        dict
    }

    #[test]
    fn parses_current_metadata_version_identifiers() {
        let mut metadata = plist::Dictionary::new();
        metadata.insert(
            "softwareVersionExternalIdentifiers".into(),
            plist::Value::Array(vec![
                plist::Value::Integer(12345678.into()),
                plist::Value::String("87654321".into()),
            ]),
        );
        metadata.insert(
            "softwareVersionExternalIdentifier".into(),
            plist::Value::Integer(87654321.into()),
        );
        let dict = response_with_song_list(plist::Value::Array(vec![version_item(metadata)]));

        let out = list_versions_from_response_dict(361309726, &dict).unwrap();

        assert_eq!(out.app_id, 361309726);
        assert_eq!(
            out.versions
                .iter()
                .map(|v| v.external_version_id.as_str())
                .collect::<Vec<_>>(),
            vec!["12345678", "87654321"]
        );
        assert_eq!(out.latest_external_version_id.as_deref(), Some("87654321"));
    }

    #[test]
    fn parses_legacy_item_version_identifiers() {
        let mut item = plist::Dictionary::new();
        item.insert(
            "externalVersionIdentifiers".into(),
            plist::Value::Array(vec![plist::Value::String("12345678".into())]),
        );
        let dict =
            response_with_song_list(plist::Value::Array(vec![plist::Value::Dictionary(item)]));

        let out = list_versions_from_response_dict(1, &dict).unwrap();

        assert_eq!(out.versions[0].external_version_id, "12345678");
        assert_eq!(out.latest_external_version_id, None);
    }

    #[test]
    fn parses_legacy_dictionary_song_list() {
        let mut song_list = plist::Dictionary::new();
        song_list.insert(
            "12345678".into(),
            plist::Value::Dictionary(plist::Dictionary::new()),
        );
        song_list.insert(
            "87654321".into(),
            plist::Value::Dictionary(plist::Dictionary::new()),
        );
        let dict = response_with_song_list(plist::Value::Dictionary(song_list));

        let out = list_versions_from_response_dict(1, &dict).unwrap();

        assert_eq!(out.versions.len(), 2);
        assert!(
            out.versions
                .iter()
                .any(|v| v.external_version_id == "12345678")
        );
        assert!(
            out.versions
                .iter()
                .any(|v| v.external_version_id == "87654321")
        );
    }

    #[test]
    fn store_error_takes_precedence() {
        let mut dict = HashMap::new();
        dict.insert("failureType".into(), plist::Value::String("2034".into()));

        let err = list_versions_from_response_dict(1, &dict).unwrap_err();

        assert!(err.is_token_expired());
    }

    #[test]
    fn license_already_exists_with_song_list_is_success() {
        let mut metadata = plist::Dictionary::new();
        metadata.insert(
            "softwareVersionExternalIdentifiers".into(),
            plist::Value::Array(vec![plist::Value::String("12345678".into())]),
        );
        let mut dict = response_with_song_list(plist::Value::Array(vec![version_item(metadata)]));
        dict.insert("failureType".into(), plist::Value::String("5002".into()));

        let out = list_versions_from_response_dict(1, &dict).unwrap();

        assert_eq!(out.versions[0].external_version_id, "12345678");
    }

    #[test]
    fn bare_license_already_exists_is_error() {
        let mut dict = HashMap::new();
        dict.insert("failureType".into(), plist::Value::String("5002".into()));

        let err = list_versions_from_response_dict(1, &dict).unwrap_err();

        assert!(err.is_license_already_exists());
    }

    #[test]
    fn missing_song_list_is_unexpected_response() {
        let dict = HashMap::new();

        let err = list_versions_from_response_dict(1, &dict).unwrap_err();

        assert!(matches!(err, ClientError::UnexpectedResponse(msg) if msg == "missing songList"));
    }

    #[test]
    fn empty_song_list_is_unexpected_response() {
        let dict = response_with_song_list(plist::Value::Array(vec![]));

        let err = list_versions_from_response_dict(1, &dict).unwrap_err();

        assert!(matches!(err, ClientError::UnexpectedResponse(msg) if msg == "empty songList"));
    }

    #[test]
    fn missing_version_identifiers_is_unexpected_response() {
        let dict = response_with_song_list(plist::Value::Array(vec![version_item(
            plist::Dictionary::new(),
        )]));

        let err = list_versions_from_response_dict(1, &dict).unwrap_err();

        assert!(
            matches!(err, ClientError::UnexpectedResponse(msg) if msg == "missing softwareVersionExternalIdentifiers")
        );
    }

    #[test]
    fn invalid_version_identifier_is_unexpected_response() {
        let mut metadata = plist::Dictionary::new();
        metadata.insert(
            "softwareVersionExternalIdentifiers".into(),
            plist::Value::Array(vec![plist::Value::Boolean(true)]),
        );
        let dict = response_with_song_list(plist::Value::Array(vec![version_item(metadata)]));

        let err = list_versions_from_response_dict(1, &dict).unwrap_err();

        assert!(
            matches!(err, ClientError::UnexpectedResponse(msg) if msg == "invalid version identifier")
        );
    }
}
