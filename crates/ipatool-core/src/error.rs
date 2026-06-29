use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("password token expired")]
    PasswordTokenExpired,
    #[error("sign-in required")]
    SignInRequired,
    #[error("license not found")]
    LicenseNotFound,
    #[error("temporarily unavailable")]
    TemporarilyUnavailable,
    #[error("purchase couldn't be completed")]
    PurchaseFailed,
    #[error("license already exists")]
    LicenseAlreadyExists,
    #[error("device verification failed")]
    DeviceVerificationFailed,
    #[error("2FA code required")]
    AuthCodeRequired,
    #[error("account disabled")]
    AccountDisabled,
    #[error("rate limited (HTTP 429)")]
    RateLimited,
    #[error("Apple store error: {code} — {message}")]
    Unknown { code: String, message: String },
}

impl StoreError {
    pub fn from_failure(failure_type: &str, customer_message: Option<&str>) -> Self {
        match failure_type {
            "-5000" => Self::InvalidCredentials,
            "2034" => Self::PasswordTokenExpired,
            "2042" => Self::SignInRequired,
            "9610" => Self::LicenseNotFound,
            // TODO: substring matching on customerMessage is locale-dependent; find a more stable signal
            "2059" => {
                let msg = customer_message.unwrap_or("");
                if msg.contains("completed") || msg.contains("not available") {
                    Self::PurchaseFailed
                } else {
                    Self::TemporarilyUnavailable
                }
            }
            "5002" => Self::LicenseAlreadyExists,
            "1008" => Self::DeviceVerificationFailed,
            "" => {
                let msg = customer_message.unwrap_or("");
                if msg.contains("BadLogin") || msg.contains("MZFinance.BadLogin") {
                    Self::AuthCodeRequired
                } else if msg.contains("disabled") {
                    Self::AccountDisabled
                } else {
                    Self::Unknown {
                        code: String::new(),
                        message: msg.to_string(),
                    }
                }
            }
            code => Self::Unknown {
                code: code.to_string(),
                message: customer_message.unwrap_or("").to_string(),
            },
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::InvalidCredentials)
    }

    pub fn is_token_expired(&self) -> bool {
        matches!(
            self,
            Self::PasswordTokenExpired | Self::SignInRequired | Self::DeviceVerificationFailed
        )
    }

    pub fn from_plist_dict(dict: &HashMap<String, plist::Value>) -> Option<Self> {
        let failure_type = dict.get("failureType").and_then(|v| match v {
            plist::Value::String(s) => Some(s.as_str()),
            plist::Value::Integer(_) => None,
            _ => None,
        });

        let failure_type_str = match &failure_type {
            Some(s) => *s,
            None => {
                if let Some(plist::Value::Integer(i)) = dict.get("failureType") {
                    return Some(Self::from_failure(
                        &i.as_signed().map_or_else(String::new, |v| v.to_string()),
                        dict.get("customerMessage").and_then(|v| v.as_string()),
                    ));
                }
                return None;
            }
        };

        Some(Self::from_failure(
            failure_type_str,
            dict.get("customerMessage").and_then(|v| v.as_string()),
        ))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    PlistDe(#[from] plist::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("missing header: {0}")]
    MissingHeader(String),
    #[error("unexpected response: {0}")]
    UnexpectedResponse(String),
}

impl ClientError {
    pub fn is_token_expired(&self) -> bool {
        matches!(self, Self::Store(e) if e.is_token_expired())
    }

    pub fn is_license_not_found(&self) -> bool {
        matches!(self, Self::Store(StoreError::LicenseNotFound))
    }

    pub fn is_license_already_exists(&self) -> bool {
        matches!(self, Self::Store(StoreError::LicenseAlreadyExists))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IpaError {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
    #[error(transparent)]
    Keyring(#[from] keyring::Error),
    #[error("app not found: {0}")]
    AppNotFound(String),
    #[error("no MAC address available")]
    NoGuid,
    #[error("{0}")]
    Other(String),
}
