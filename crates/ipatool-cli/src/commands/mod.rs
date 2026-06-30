pub mod auth;
pub mod download;
pub mod purchase;
pub mod search;
pub mod version;

use anyhow::{Context, Result};

use ipatool_core::client::AppleClient;
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
