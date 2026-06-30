use std::collections::HashMap;

use serde::Serialize;
use time::format_description::well_known::Rfc3339;

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

    let info = crate::ipa::partial_zip::read_remote_info_plist(client.http(), &item.url)
        .await
        .map_err(|e| {
            ClientError::UnexpectedResponse(format!("failed to read version metadata: {e}"))
        })?;

    version_metadata_from_info_plist(&info.plist, info.modified, item.metadata)
}

fn download_url(account: &Account, guid: &str) -> String {
    let host = match &account.pod {
        Some(pod) => format!("p{pod}-buy.itunes.apple.com"),
        None => "buy.itunes.apple.com".to_string(),
    };
    format!("https://{host}/WebObjects/MZFinance.woa/wa/volumeStoreDownloadProduct?guid={guid}")
}

fn version_metadata_from_info_plist(
    info: &plist::Dictionary,
    modified: Option<zip::DateTime>,
    extra: HashMap<String, plist::Value>,
) -> Result<VersionMetadata, ClientError> {
    let bundle_short_version = first_metadata_string(
        info,
        &["CFBundleShortVersionString", "bundleShortVersionString"],
    );
    let bundle_version = first_metadata_string(info, &["CFBundleVersion", "bundleVersion"]);
    let release_date = release_date_from_info_plist(info, modified)?;

    Ok(VersionMetadata {
        bundle_short_version,
        bundle_version,
        release_date,
        extra,
    })
}

fn first_metadata_string(info: &plist::Dictionary, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| info.get(key).and_then(metadata_value_to_string))
}

fn metadata_value_to_string(value: &plist::Value) -> Option<String> {
    let value = match value {
        plist::Value::String(s) => s.trim().to_string(),
        plist::Value::Integer(i) => i
            .as_signed()
            .map(|n| n.to_string())
            .or_else(|| i.as_unsigned().map(|n| n.to_string()))?,
        plist::Value::Real(n) => n.to_string(),
        _ => return None,
    };

    if value.is_empty() { None } else { Some(value) }
}

fn release_date_from_info_plist(
    info: &plist::Dictionary,
    modified: Option<zip::DateTime>,
) -> Result<Option<String>, ClientError> {
    for key in ["releaseDate", "ReleaseDate"] {
        if let Some(value) = info.get(key) {
            return parse_release_date_value(value).map(Some);
        }
    }

    Ok(modified.map(zip_datetime_to_rfc3339))
}

fn parse_release_date_value(value: &plist::Value) -> Result<String, ClientError> {
    match value {
        plist::Value::Date(date) => Ok(date.to_xml_format()),
        plist::Value::String(s) => parse_release_date_string(s),
        plist::Value::Integer(i) => {
            let timestamp = i.as_signed().or_else(|| {
                i.as_unsigned()
                    .and_then(|n| (n <= i64::MAX as u64).then_some(n as i64))
            });
            let timestamp = timestamp.ok_or_else(|| {
                ClientError::UnexpectedResponse("release date timestamp is too large".into())
            })?;
            unix_timestamp_to_rfc3339(timestamp)
        }
        plist::Value::Real(n) if n.is_finite() => unix_timestamp_to_rfc3339(*n as i64),
        plist::Value::Real(_) => Err(ClientError::UnexpectedResponse(
            "release date timestamp is not finite".into(),
        )),
        _ => Err(ClientError::UnexpectedResponse(format!(
            "unsupported release date type: {value:?}"
        ))),
    }
}

fn parse_release_date_string(value: &str) -> Result<String, ClientError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ClientError::UnexpectedResponse(
            "release date is empty".into(),
        ));
    }

    if let Ok(parsed) = time::OffsetDateTime::parse(value, &Rfc3339) {
        return format_datetime(parsed);
    }

    if is_date_only(value) {
        return Ok(format!("{value}T00:00:00Z"));
    }

    Err(ClientError::UnexpectedResponse(format!(
        "invalid release date: {value}"
    )))
}

fn is_date_only(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

fn unix_timestamp_to_rfc3339(timestamp: i64) -> Result<String, ClientError> {
    let datetime = time::OffsetDateTime::from_unix_timestamp(timestamp)
        .map_err(|e| ClientError::UnexpectedResponse(format!("invalid release date: {e}")))?;
    format_datetime(datetime)
}

fn format_datetime(datetime: time::OffsetDateTime) -> Result<String, ClientError> {
    datetime
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .map_err(|e| ClientError::UnexpectedResponse(format!("format release date: {e}")))
}

fn zip_datetime_to_rfc3339(datetime: zip::DateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        datetime.year(),
        datetime.month(),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    )
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

    fn stale_download_metadata() -> HashMap<String, plist::Value> {
        HashMap::from([
            (
                "bundleShortVersionString".into(),
                plist::Value::String("1.0".into()),
            ),
            ("bundleVersion".into(), plist::Value::String("100".into())),
            (
                "releaseDate".into(),
                plist::Value::String("2010-04-01T20:36:57Z".into()),
            ),
        ])
    }

    #[test]
    fn version_metadata_uses_info_plist_values_not_download_metadata() {
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleShortVersionString".into(),
            plist::Value::String("2.0".into()),
        );
        info.insert("CFBundleVersion".into(), plist::Value::String("200".into()));
        info.insert(
            "releaseDate".into(),
            plist::Value::String("2024-04-02T12:00:00Z".into()),
        );

        let out = version_metadata_from_info_plist(&info, None, stale_download_metadata()).unwrap();

        assert_eq!(out.bundle_short_version.as_deref(), Some("2.0"));
        assert_eq!(out.bundle_version.as_deref(), Some("200"));
        assert_eq!(out.release_date.as_deref(), Some("2024-04-02T12:00:00Z"));
        assert_eq!(
            out.extra.get("releaseDate").and_then(|v| v.as_string()),
            Some("2010-04-01T20:36:57Z")
        );
    }

    #[test]
    fn version_metadata_falls_back_to_info_plist_zip_time() {
        let mut info = plist::Dictionary::new();
        info.insert(
            "CFBundleShortVersionString".into(),
            plist::Value::String("2.0".into()),
        );
        let modified = zip::DateTime::from_date_and_time(2024, 3, 19, 12, 0, 0).unwrap();

        let out =
            version_metadata_from_info_plist(&info, Some(modified), stale_download_metadata())
                .unwrap();

        assert_eq!(out.release_date.as_deref(), Some("2024-03-19T12:00:00Z"));
    }

    #[test]
    fn invalid_info_plist_release_date_is_an_error() {
        let mut info = plist::Dictionary::new();
        info.insert(
            "releaseDate".into(),
            plist::Value::String("not-a-date".into()),
        );

        let err = version_metadata_from_info_plist(&info, None, HashMap::new()).unwrap_err();

        assert!(
            matches!(err, ClientError::UnexpectedResponse(msg) if msg == "invalid release date: not-a-date")
        );
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
