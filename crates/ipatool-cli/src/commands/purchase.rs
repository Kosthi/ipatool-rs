use anyhow::{Context, Result};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::model::storefront::country_code_from_store_front;
use ipatool_core::model::{Account, Platform};

pub async fn purchase(
    client: &AppleClient,
    bundle_identifier: &str,
    account: &Account,
    _format: crate::output::OutputFormat,
) -> Result<()> {
    let country = country_code_from_store_front(&account.store_front).unwrap_or("US");

    let app = api::lookup::lookup(client, bundle_identifier, country, Platform::IPhone)
        .await
        .context("lookup failed")?
        .ok_or_else(|| anyhow::anyhow!("app not found: {bundle_identifier}"))?;

    eprintln!("Purchasing {} ({})", app.name, app.id);

    match api::purchase::purchase(client, app.id, account).await {
        Ok(()) => {}
        Err(e) if e.is_token_expired() => {
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
