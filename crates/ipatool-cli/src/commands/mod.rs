pub mod auth;
pub mod download;
pub mod purchase;
pub mod search;
pub mod version;

use anyhow::{Context, Result};

use ipatool_core::client::AppleClient;
use ipatool_core::error::ClientError;
use ipatool_core::model::Account;

pub async fn reauth_or_fail(client: &AppleClient, account: &Account) -> Result<Account> {
    let new_account = ipatool_core::api::reauth::reauthenticate(client, account)
        .await
        .context("re-authentication failed")?;
    ipatool_core::credential::store_account(&new_account)
        .context("failed to store refreshed credentials")?;
    eprintln!("Re-authenticated as {}", new_account.name);
    Ok(new_account)
}

pub fn app_not_found_error(bundle_identifier: &str, country: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "app not found for bundle identifier {bundle_identifier} in storefront {country}; check -b/--bundle-identifier"
    )
}

pub fn is_empty_song_list_error(error: &ClientError) -> bool {
    matches!(error, ClientError::UnexpectedResponse(msg) if msg == "empty songList")
}

pub fn version_not_found_error(
    version_id: &str,
    app_id: i64,
    bundle_identifier: Option<&str>,
) -> anyhow::Error {
    let target = match bundle_identifier {
        Some(bid) => format!("{bid} (app id: {app_id})"),
        None => format!("app id {app_id}"),
    };
    let list_hint = match bundle_identifier {
        Some(bid) => format!("ipatool version list -b {bid}"),
        None => format!("ipatool version list -i {app_id}"),
    };

    anyhow::anyhow!(
        "version id {version_id} was not found for {target}; run `{list_hint}` to see available versions"
    )
}

#[cfg(test)]
mod tests {
    use ipatool_core::error::ClientError;

    #[test]
    fn empty_song_list_detection_is_exact() {
        let empty = ClientError::UnexpectedResponse("empty songList".into());
        let other = ClientError::UnexpectedResponse("missing songList".into());

        assert!(super::is_empty_song_list_error(&empty));
        assert!(!super::is_empty_song_list_error(&other));
    }

    #[test]
    fn app_not_found_mentions_bundle_storefront_and_flag() {
        let err = super::app_not_found_error("com.example.missing", "US");

        assert_eq!(
            err.to_string(),
            "app not found for bundle identifier com.example.missing in storefront US; check -b/--bundle-identifier"
        );
    }

    #[test]
    fn version_not_found_uses_bundle_specific_hint() {
        let err = super::version_not_found_error("887118712", 432274380, Some("com.zhihu.ios"));

        assert_eq!(
            err.to_string(),
            "version id 887118712 was not found for com.zhihu.ios (app id: 432274380); run `ipatool version list -b com.zhihu.ios` to see available versions"
        );
    }
}
