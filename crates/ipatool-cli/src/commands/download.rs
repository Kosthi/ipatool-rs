#![allow(clippy::too_many_arguments)]

use std::path::PathBuf;

use anyhow::{Context, Result};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::ipa::patch;
use ipatool_core::model::storefront::country_code_from_store_front;
use ipatool_core::model::{Account, Platform};

use super::reauth_or_fail;

const MAX_ATTEMPTS: u32 = 3;

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
    let mut account = account.clone();
    let country = country_code_from_store_front(&account.store_front).unwrap_or("US");

    let resolved_app_id = match app_id {
        Some(id) => id,
        None => {
            let bid = bundle_identifier.ok_or_else(|| {
                anyhow::anyhow!("either --bundle-identifier or --app-id is required")
            })?;
            let app = api::lookup::lookup(client, bid, country, platform)
                .await
                .context("lookup failed")?
                .ok_or_else(|| anyhow::anyhow!("app not found: {bid}"))?;
            eprintln!("Found: {} ({})", app.name, app.id);
            app.id
        }
    };

    eprintln!("Requesting download info...");
    let mut item = None;
    let mut purchase_attempted = false;
    let mut last_download_error = None;

    for attempt in 0..MAX_ATTEMPTS {
        match api::download::get_download_info(client, resolved_app_id, &account, version_id).await
        {
            Ok(i) => {
                item = Some(i);
                break;
            }
            Err(e) if e.is_license_not_found() && do_purchase && !purchase_attempted => {
                last_download_error = Some(e.to_string());
                eprintln!("License not found, purchasing...");
                purchase_for_download(client, resolved_app_id, &mut account).await?;
                purchase_attempted = true;
                eprintln!("Purchase successful");
            }
            Err(e) if e.is_license_not_found() && do_purchase => {
                return Err(e).context("license not found after purchase");
            }
            Err(e) if e.is_token_expired() && attempt + 1 < MAX_ATTEMPTS => {
                last_download_error = Some(e.to_string());
                eprintln!(
                    "Token expired, re-authenticating (attempt {})...",
                    attempt + 1
                );
                account = reauth_or_fail(client, &account).await?;
            }
            Err(e) if e.is_license_not_found() => {
                return Err(e).context("license not found (use --purchase to acquire)");
            }
            Err(e) => {
                return Err(e).context("failed to get download info");
            }
        }
    }

    let item = item.with_context(|| {
        last_download_error.map_or_else(
            || "failed to get download info after retries".to_string(),
            |e| format!("failed to get download info after retries: {e}"),
        )
    })?;

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
    patch::patch_ipa(&tmp_path, &dest, &item, &account.email).context("failed to patch IPA")?;

    std::fs::remove_file(&tmp_path).ok();

    eprintln!("Saved to {}", dest.display());
    Ok(())
}

async fn purchase_for_download(
    client: &AppleClient,
    app_id: i64,
    account: &mut Account,
) -> Result<()> {
    match api::purchase::purchase(client, app_id, account).await {
        Ok(()) => Ok(()),
        Err(e) if e.is_token_expired() => {
            *account = reauth_or_fail(client, account).await?;
            api::purchase::purchase(client, app_id, account)
                .await
                .context("purchase failed after re-auth")
        }
        Err(e) if e.is_license_already_exists() => Ok(()),
        Err(e) => Err(e).context("purchase failed"),
    }
}
