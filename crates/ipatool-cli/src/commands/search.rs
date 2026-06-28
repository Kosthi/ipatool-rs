use anyhow::{Context, Result};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::model::Platform;

use crate::output::{OutputFormat, print_apps};

pub async fn search(
    client: &AppleClient,
    term: &str,
    limit: u32,
    platform: Platform,
    country: &str,
    format: OutputFormat,
) -> Result<()> {
    let apps = api::search::search(client, term, country, platform, limit)
        .await
        .context("search failed")?;

    if apps.is_empty() {
        eprintln!("No results found");
    } else {
        print_apps(&apps, format);
    }

    Ok(())
}
