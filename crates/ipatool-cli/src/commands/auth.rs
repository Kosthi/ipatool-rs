use anyhow::{Context, Result, bail};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::credential;

use crate::output::{OutputFormat, print_account};

pub async fn login(
    client: &mut AppleClient,
    email: &str,
    password: Option<&str>,
    auth_code: Option<&str>,
    non_interactive: bool,
    format: OutputFormat,
) -> Result<()> {
    let password = match password {
        Some(p) => p.to_string(),
        None => {
            if non_interactive {
                bail!("password required in non-interactive mode");
            }
            rpassword::prompt_password("Password: ").context("failed to read password")?
        }
    };

    let auth_url = api::bag::fetch_auth_endpoint(client)
        .await
        .context("failed to fetch auth endpoint")?;

    tracing::info!(url = %auth_url, "using auth endpoint");

    let account = api::auth::login(client, email, &password, auth_code, &auth_url)
        .await
        .context("login failed")?;

    credential::store_account(&account).context("failed to store credentials")?;

    client.set_account(account.clone());

    eprintln!("Logged in as {}", account.name);
    print_account(&account, format);
    Ok(())
}

pub async fn info(format: OutputFormat) -> Result<()> {
    let account = credential::load_account()
        .context("failed to load credentials")?
        .context("not logged in, run `auth login` first")?;

    print_account(&account, format);
    Ok(())
}

pub async fn revoke() -> Result<()> {
    credential::delete_account().context("failed to delete credentials")?;

    eprintln!("Credentials removed");
    Ok(())
}
