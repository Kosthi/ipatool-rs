use crate::client::AppleClient;
use crate::error::ClientError;
use crate::model::Account;

pub async fn reauthenticate(
    client: &AppleClient,
    account: &Account,
) -> Result<Account, ClientError> {
    let password = account
        .password
        .as_deref()
        .ok_or_else(|| {
            ClientError::UnexpectedResponse(
                "stored credentials do not include a password; run `ipatool auth login` to refresh the session".into(),
            )
        })?;

    let auth_url = super::bag::fetch_auth_endpoint(client).await?;

    tracing::info!("re-authenticating as {}", account.email);

    let mut new_account =
        super::auth::login(client, &account.email, password, None, &auth_url).await?;

    new_account.password = account.password.clone();
    Ok(new_account)
}
