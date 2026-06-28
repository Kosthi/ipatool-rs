use anyhow::{Context, Result};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::model::{Account, Platform};
use ipatool_core::model::storefront::country_code_from_store_front;

use crate::output::{self, OutputFormat};

pub async fn list(
    client: &AppleClient,
    app_id: Option<i64>,
    bundle_identifier: Option<&str>,
    account: &Account,
    format: OutputFormat,
) -> Result<()> {
    let resolved_app_id = resolve_app_id(client, app_id, bundle_identifier, account).await?;

    let result = api::versions::list_versions(client, resolved_app_id, account)
        .await
        .context("failed to list versions")?;

    match format {
        OutputFormat::Text => {
            eprintln!("Versions for app {}:", result.app_id);
            for v in &result.versions {
                let ver = v.version_string.as_deref().unwrap_or("?");
                println!("  {} (version: {})", v.external_version_id, ver);
            }
            if result.versions.is_empty() {
                eprintln!("  (no versions found)");
            }
        }
        OutputFormat::Json => output::print_json(&result),
    }

    Ok(())
}

pub async fn meta(
    client: &AppleClient,
    app_id: Option<i64>,
    bundle_identifier: Option<&str>,
    version_id: &str,
    account: &Account,
    format: OutputFormat,
) -> Result<()> {
    let resolved_app_id = resolve_app_id(client, app_id, bundle_identifier, account).await?;

    let result =
        api::versions::get_version_metadata(client, resolved_app_id, account, version_id)
            .await
            .context("failed to get version metadata")?;

    match format {
        OutputFormat::Text => {
            if let Some(ref v) = result.bundle_short_version {
                println!("Version:       {v}");
            }
            if let Some(ref v) = result.bundle_version {
                println!("Build:         {v}");
            }
            if let Some(ref v) = result.release_date {
                println!("Release Date:  {v}");
            }
        }
        OutputFormat::Json => output::print_json(&result),
    }

    Ok(())
}

async fn resolve_app_id(
    client: &AppleClient,
    app_id: Option<i64>,
    bundle_identifier: Option<&str>,
    account: &Account,
) -> Result<i64> {
    match app_id {
        Some(id) => Ok(id),
        None => {
            let bid = bundle_identifier
                .ok_or_else(|| anyhow::anyhow!("either --app-id or --bundle-identifier is required"))?;
            let country =
                country_code_from_store_front(&account.store_front).unwrap_or("US");
            let app = api::lookup::lookup(client, bid, country, Platform::IPhone)
                .await
                .context("lookup failed")?
                .ok_or_else(|| anyhow::anyhow!("app not found: {bid}"))?;
            Ok(app.id)
        }
    }
}
