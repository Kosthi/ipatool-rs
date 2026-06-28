use crate::error::IpaError;
use crate::model::account::Account;

const SERVICE_NAME: &str = "ipatool-rs";
const ACCOUNT_KEY: &str = "account";

pub fn store_account(account: &Account) -> Result<(), IpaError> {
    let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_KEY)?;
    let json = serde_json::to_string(account)
        .map_err(|e| IpaError::Other(format!("failed to serialize account: {e}")))?;
    entry.set_password(&json).map_err(IpaError::Keyring)?;
    Ok(())
}

pub fn load_account() -> Result<Option<Account>, IpaError> {
    let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_KEY)?;
    match entry.get_password() {
        Ok(json) => {
            let account: Account = serde_json::from_str(&json)
                .map_err(|e| IpaError::Other(format!("failed to parse stored account: {e}")))?;
            Ok(Some(account))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(IpaError::Keyring(e)),
    }
}

pub fn delete_account() -> Result<(), IpaError> {
    let entry = keyring::Entry::new(SERVICE_NAME, ACCOUNT_KEY)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(IpaError::Keyring(e)),
    }
}
