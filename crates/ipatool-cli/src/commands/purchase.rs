use anyhow::{Context, Result};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::model::storefront::country_code_from_store_front;
use ipatool_core::model::{Account, Platform};

use super::app_not_found_error;

pub async fn purchase(
    client: &AppleClient,
    bundle_identifier: &str,
    account: &Account,
    _format: crate::output::OutputFormat,
) -> Result<()> {
    let country = country_code_from_store_front(&account.store_front).unwrap_or("US");

    let app = api::lookup::lookup(client, bundle_identifier, country, Platform::IPhone)
        .await
        .with_context(|| {
            format!(
                "lookup failed for bundle identifier {bundle_identifier} in storefront {country}"
            )
        })?
        .ok_or_else(|| app_not_found_error(bundle_identifier, country))?;

    eprintln!("Purchasing {} (app id: {})", app.name, app.id);

    match api::purchase::purchase(client, app.id, account).await {
        Ok(()) => {}
        Err(e) if e.is_token_expired() => {
            eprintln!("Token expired, re-authenticating...");
            let new_account = super::reauth_or_fail(client, account).await?;
            api::purchase::purchase(client, app.id, &new_account)
                .await
                .context("purchase failed after re-auth")?;
        }
        Err(e) => return Err(e).context("purchase failed"),
    }

    eprintln!("Done");
    Ok(())
}
