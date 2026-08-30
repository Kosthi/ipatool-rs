use anyhow::{Context, Result, bail};

use ipatool_core::api;
use ipatool_core::client::AppleClient;
use ipatool_core::credential;
use ipatool_core::error::{ClientError, StoreError};

use crate::output::{OutputFormat, print_account};
use crate::read_password;

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
            read_password::prompt_password("Password: ").context("failed to read password")?
        }
    };

    let bag = api::bag::fetch_bag(client)
        .await
        .context("failed to fetch bag")?;

    tracing::info!(url = %bag.auth_endpoint, "using auth endpoint");

    // Established once and reused: the handshake costs two round-trips to Apple
    // and, on a cold cache, a download of the Apple frameworks and the emulator.
    let signer = ipatool_core::sap::new_default_signer(client, &bag.sap, client.hardware_id())
        .await
        .context("failed to establish a SAP signing session")?;
    let signer = Some(&signer as &dyn ipatool_core::sap::ActionSigner);

    let mut account = match api::auth::login(
        client,
        email,
        &password,
        auth_code,
        &bag.auth_endpoint,
        signer,
    )
    .await
    {
        Ok(account) => account,
        Err(ClientError::Store(StoreError::AuthCodeRequired))
            if auth_code.is_none() && !non_interactive =>
        {
            eprintln!(
                "Apple rejected the login. If Apple is showing a two-factor code, enter it now; otherwise press Enter and check your password."
            );
            let auth_code = read_password::prompt_visible("Two-factor code (optional): ")
                .context("failed to read 2FA code")?;
            let auth_code = auth_code.trim();
            if auth_code.is_empty() {
                bail!(
                    "login rejected; check your password, or rerun with --auth-code <code> if Apple is asking for two-factor authentication"
                );
            }

            match api::auth::login(
                client,
                email,
                &password,
                Some(auth_code),
                &bag.auth_endpoint,
                signer,
            )
            .await
            {
                Ok(account) => account,
                Err(ClientError::Store(StoreError::AuthCodeRequired)) => {
                    bail!("login rejected; check your password and 2FA code")
                }
                Err(e) => return Err(e).context("login failed"),
            }
        }
        Err(ClientError::Store(StoreError::AuthCodeRequired)) if auth_code.is_none() => {
            bail!(
                "login rejected; check your password, or rerun with --auth-code <code> if Apple is asking for two-factor authentication"
            )
        }
        Err(ClientError::Store(StoreError::AuthCodeRequired)) => {
            bail!("login rejected; check your password and 2FA code")
        }
        Err(e) => return Err(e).context("login failed"),
    };

    account.password = Some(password);
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
