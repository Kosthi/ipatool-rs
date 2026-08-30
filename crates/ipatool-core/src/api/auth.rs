use std::collections::HashMap;
use std::time::Duration;
use url::Url;

use crate::client::AppleClient;
use crate::error::{ClientError, StoreError};
use crate::model::Account;
use crate::sap::ActionSigner;

const MAX_ATTEMPTS: u32 = 4;
const MAX_REDIRECTS: u32 = 5;
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Header carrying the SAP signature over the request body.
const ACTION_SIGNATURE_HEADER: &str = "X-Apple-ActionSignature";

pub async fn login(
    client: &AppleClient,
    email: &str,
    password: &str,
    auth_code: Option<&str>,
    auth_url: &Url,
    signer: Option<&dyn ActionSigner>,
) -> Result<Account, ClientError> {
    super::bag::validate_auth_endpoint(auth_url)?;

    let password_with_code = match auth_code {
        Some(code) => format!("{password}{code}"),
        None => password.to_string(),
    };

    let mut attempt = 1u32;
    let mut redirects = 0u32;
    let mut current_url = auth_url.clone();

    loop {
        let body = build_auth_plist(email, &password_with_code, client.guid(), attempt);
        let mut body_bytes = Vec::new();
        plist::to_writer_xml(&mut body_bytes, &body)
            .map_err(|e| ClientError::UnexpectedResponse(format!("plist serialize: {e}")))?;

        tracing::debug!(attempt, url = %current_url, signed = signer.is_some(), "sending auth request");

        let mut request = client
            .http()
            .post(current_url.as_str())
            .header("Content-Type", "application/x-apple-plist")
            .timeout(AUTH_REQUEST_TIMEOUT);

        // Apple signs the exact bytes it receives, so this has to happen after
        // the body is serialized and before it is sent.
        if let Some(signer) = signer {
            let signature = signer.sign(&body_bytes)?;
            request = request.header(ACTION_SIGNATURE_HEADER, encode_base64(&signature));
        }

        let resp = request.body(body_bytes).send().await?;

        let status = resp.status();
        tracing::debug!(%status, "auth response status");

        if status.is_redirection()
            && let Some(location) = resp.headers().get("location")
        {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(ClientError::UnexpectedResponse(
                    "too many auth redirects".into(),
                ));
            }

            let new_url = location
                .to_str()
                .map_err(|_| ClientError::MissingHeader("location (invalid)".into()))?;
            tracing::debug!(new_url, "following redirect");
            let new_url = current_url
                .join(new_url)
                .map_err(|e| ClientError::UnexpectedResponse(format!("redirect URL: {e}")))?;

            // The redirect target receives the credentials on the next pass, so
            // it gets the same scrutiny as the endpoint from the bag.
            super::bag::validate_auth_endpoint(&new_url)?;
            current_url = new_url;
            continue;
        }

        let store_front = resp
            .headers()
            .get("x-set-apple-store-front")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let pod = resp
            .headers()
            .get("pod")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let resp_body = resp.bytes().await?;

        tracing::debug!(
            len = resp_body.len(),
            preview = %String::from_utf8_lossy(&resp_body[..resp_body.len().min(500)]),
            "auth response body"
        );

        if resp_body.is_empty() {
            // Apple's application tier drops unsigned sign-in requests with a
            // zero-length 403. Saying so beats reporting an empty response.
            if signer.is_none() && status == reqwest::StatusCode::FORBIDDEN {
                return Err(ClientError::SapSignatureRequired {
                    status: status.as_u16(),
                });
            }

            return Err(ClientError::UnexpectedResponse(format!(
                "empty response (HTTP {status})"
            )));
        }

        let dict: HashMap<String, plist::Value> =
            crate::client::plist_xml::parse_plist_response(&resp_body)?;

        if let Some(err) = StoreError::from_plist_dict(&dict) {
            if err.is_retryable() && attempt < MAX_ATTEMPTS {
                attempt += 1;
                tracing::warn!("retryable error, attempt {attempt}");
                continue;
            }
            return Err(ClientError::Store(err));
        }

        let password_token = dict
            .get("passwordToken")
            .and_then(|v| v.as_string())
            .ok_or_else(|| ClientError::UnexpectedResponse("missing passwordToken".into()))?
            .to_string();

        let ds_person_id = dict
            .get("dsPersonId")
            .map(|v| match v {
                plist::Value::String(s) => s.clone(),
                plist::Value::Integer(i) => {
                    i.as_signed().map_or_else(String::new, |n| n.to_string())
                }
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ClientError::UnexpectedResponse("missing dsPersonId".into()))?;

        let name = dict
            .get("accountInfo")
            .and_then(|v| v.as_dictionary())
            .and_then(|d| d.get("address"))
            .and_then(|v| v.as_dictionary())
            .and_then(|d| {
                let first = d.get("firstName")?.as_string()?;
                let last = d.get("lastName")?.as_string()?;
                Some(format!("{first} {last}"))
            })
            .unwrap_or_default();

        let sf = store_front.unwrap_or_default();

        return Ok(Account {
            email: email.to_string(),
            password_token,
            directory_services_id: ds_person_id,
            name,
            store_front: sf,
            pod,
            password: None,
        });
    }
}

fn encode_base64(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn build_auth_plist(email: &str, password: &str, guid: &str, attempt: u32) -> plist::Dictionary {
    let mut dict = plist::Dictionary::new();
    dict.insert("appleId".into(), plist::Value::String(email.into()));
    dict.insert("attempt".into(), plist::Value::String(attempt.to_string()));
    dict.insert("guid".into(), plist::Value::String(guid.into()));
    dict.insert("password".into(), plist::Value::String(password.into()));
    dict.insert("rmp".into(), plist::Value::String("0".into()));
    dict.insert("why".into(), plist::Value::String("signIn".into()));
    dict
}
