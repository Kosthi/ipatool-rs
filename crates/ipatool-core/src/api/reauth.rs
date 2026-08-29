use crate::client::AppleClient;
use crate::error::ClientError;
use crate::model::Account;
use crate::sap;

pub async fn reauthenticate(
    client: &AppleClient,
    account: &Account,
) -> Result<Account, ClientError> {
    let password = account
        .password
        .as_deref()
        .ok_or_else(|| ClientError::UnexpectedResponse("no stored password for re-auth".into()))?;

    let bag = super::bag::fetch_bag(client).await?;

    let signer = sap::new_default_signer(client, &bag.sap, client.hardware_id()).await?;

    tracing::info!("re-authenticating as {}", account.email);

    let mut new_account = super::auth::login(
        client,
        &account.email,
        password,
        None,
        &bag.auth_endpoint,
        Some(&signer),
    )
    .await?;

    new_account.password = account.password.clone();
    Ok(new_account)
}
