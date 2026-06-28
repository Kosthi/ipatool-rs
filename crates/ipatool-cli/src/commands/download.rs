use std::path::PathBuf;

use anyhow::{Context, Result};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::ipa::patch;
use ipatool_core::model::{Account, Platform};
use ipatool_core::model::storefront::country_code_from_store_front;

pub async fn download(
    client: &AppleClient,
    bundle_identifier: Option<&str>,
    app_id: Option<i64>,
    version_id: Option<&str>,
    output: Option<PathBuf>,
    do_purchase: bool,
    platform: Platform,
    account: &Account,
) -> Result<()> {
    let country = country_code_from_store_front(&account.store_front).unwrap_or("US");

    let resolved_app_id = match app_id {
        Some(id) => id,
        None => {
            let bid = bundle_identifier
                .ok_or_else(|| anyhow::anyhow!("either --bundle-identifier or --app-id is required"))?;
            let app = api::lookup::lookup(client, bid, country, platform)
                .await
                .context("lookup failed")?
                .ok_or_else(|| anyhow::anyhow!("app not found: {bid}"))?;
            eprintln!("Found: {} ({})", app.name, app.id);
            app.id
        }
    };

    if do_purchase {
        eprintln!("Obtaining license...");
        api::purchase::purchase(client, resolved_app_id, account)
            .await
            .context("purchase failed")?;
    }

    eprintln!("Requesting download info...");
    let item =
        api::download::get_download_info(client, resolved_app_id, account, version_id)
            .await
            .context("failed to get download info")?;

    let version = item
        .metadata
        .get("bundleShortVersionString")
        .and_then(|v| v.as_string())
        .unwrap_or("unknown");

    let bid = bundle_identifier.unwrap_or("app");
    let filename = format!("{bid}_{resolved_app_id}_{version}.ipa");
    let dest = output.unwrap_or_else(|| PathBuf::from(&filename));
    let tmp_path = dest.with_extension("ipa.tmp");

    eprintln!("Downloading to {}", tmp_path.display());
    api::download::download_file(client, &item.url, &tmp_path, true)
        .await
        .context("download failed")?;

    eprintln!("Patching IPA...");
    patch::patch_ipa(&tmp_path, &dest, &item, &account.email)
        .context("failed to patch IPA")?;

    std::fs::remove_file(&tmp_path).ok();

    eprintln!("Saved to {}", dest.display());
    Ok(())
}
