use std::collections::HashMap;
use std::time::Duration;
use url::Url;

use crate::client::AppleClient;
use crate::client::plist_xml::{looks_like_plist, parse_plist_response};
use crate::error::{ClientError, StoreError};
use crate::model::Account;
use crate::sap::ActionSigner;

const MAX_ATTEMPTS: u32 = 4;
const MAX_REDIRECTS: u32 = 5;
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Apple's edge drops sign-in requests on its own account, answering with a
/// contentless 204, an HTML 404, a bare 3xx carrying no `Location`, or a 5xx —
/// none of which say anything about the credentials. The same bytes go through
/// moments later, and which endpoint misbehaves even moves around between
/// tries, so resending is worth more than any of it is diagnostic.
/// See issue #17 and majd/ipatool#520.
const MAX_SEND_TRIES: u32 = 3;
const SEND_RETRY_DELAY: Duration = Duration::from_millis(250);

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

        // Apple signs the exact bytes it receives, so this has to happen after
        // the body is serialized and before it is sent.
        let signature = match signer {
            Some(signer) => Some(encode_base64(&signer.sign(&body_bytes)?)),
            None => None,
        };

        tracing::debug!(attempt, url = %current_url, signed = signature.is_some(), "sending auth request");

        let resp =
            send_auth_request(client, &current_url, &body_bytes, signature.as_deref()).await?;
        let status = resp.status;

        if status.is_redirection()
            && let Some(location) = resp.location.as_deref()
        {
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Err(ClientError::UnexpectedResponse(
                    "too many auth redirects".into(),
                ));
            }

            tracing::debug!(location, "following redirect");
            let new_url = current_url
                .join(location)
                .map_err(|e| ClientError::UnexpectedResponse(format!("redirect URL: {e}")))?;

            // The redirect target receives the credentials on the next pass, so
            // it gets the same scrutiny as the endpoint from the bag.
            super::bag::validate_auth_endpoint(&new_url)?;
            current_url = new_url;
            continue;
        }

        if resp.body.is_empty() {
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

        if !looks_like_plist(&resp.body) {
            // Without this the body reaches the plist parser and the failure
            // surfaces as `invalid type: string "<html>"`, which says nothing.
            return Err(ClientError::UnexpectedResponse(format!(
                "HTTP {status}: response is not a property list: {}",
                String::from_utf8_lossy(&resp.body[..resp.body.len().min(200)]).trim()
            )));
        }

        let dict: HashMap<String, plist::Value> = parse_plist_response(&resp.body)?;

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

        return Ok(Account {
            email: email.to_string(),
            password_token,
            directory_services_id: ds_person_id,
            name,
            store_front: resp.store_front.unwrap_or_default(),
            pod: resp.pod,
            password: None,
        });
    }
}

/// One completed sign-in exchange, read into memory so that both the
/// dropped-request check and the caller can look at the body.
struct AuthResponse {
    status: reqwest::StatusCode,
    location: Option<String>,
    store_front: Option<String>,
    pod: Option<String>,
    body: Vec<u8>,
}

/// Sends the sign-in POST, resending the identical bytes while Apple answers
/// with something that is about its own edge rather than about the account.
async fn send_auth_request(
    client: &AppleClient,
    url: &Url,
    body: &[u8],
    signature: Option<&str>,
) -> Result<AuthResponse, ClientError> {
    let mut statuses: Vec<String> = Vec::new();
    let mut send = 1u32;

    loop {
        let mut request = client
            .http()
            .post(url.as_str())
            .header("Content-Type", "application/x-apple-plist")
            .timeout(AUTH_REQUEST_TIMEOUT)
            .body(body.to_vec());

        if let Some(signature) = signature {
            request = request.header(ACTION_SIGNATURE_HEADER, signature);
        }

        let raw = request.send().await?;

        let status = raw.status();
        let location = header(&raw, "location");
        let store_front = header(&raw, "x-set-apple-store-front");
        let pod = header(&raw, "pod");
        let body = raw.bytes().await?.to_vec();

        tracing::debug!(
            send,
            %status,
            len = body.len(),
            preview = %String::from_utf8_lossy(&body[..body.len().min(500)]),
            "auth response"
        );

        if !is_dropped_request(status, location.is_some(), &body) {
            return Ok(AuthResponse {
                status,
                location,
                store_front,
                pod,
                body,
            });
        }

        statuses.push(status.as_u16().to_string());

        if send == MAX_SEND_TRIES {
            return Err(ClientError::AuthEndpointUnavailable {
                tries: MAX_SEND_TRIES,
                statuses: statuses.join(", "),
            });
        }

        tracing::warn!(%status, "Apple returned no store response, resending the sign-in request");
        tokio::time::sleep(SEND_RETRY_DELAY * send).await;
        send += 1;
    }
}

/// Whether a response is Apple's edge dropping the request rather than an
/// answer about the account.
///
/// A 3xx carrying a `Location` is a real pod redirect and the caller follows
/// it; a bare one is not. A body that parses as a property list is a store
/// reply whatever its status, so it is never thrown away — Apple does report
/// real failures under a 5xx.
fn is_dropped_request(status: reqwest::StatusCode, has_location: bool, body: &[u8]) -> bool {
    if looks_like_plist(body) {
        return false;
    }

    match status.as_u16() {
        204 | 404 => true,
        _ if status.is_server_error() => true,
        _ if status.is_redirection() => !has_location,
        _ => false,
    }
}

fn header(resp: &reqwest::Response, name: &str) -> Option<String> {
    match resp.headers().get(name)?.to_str() {
        Ok(value) => Some(value.to_string()),
        Err(_) => {
            tracing::warn!(
                header = name,
                "response header is not valid UTF-8, ignoring"
            );
            None
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    const STORE_REPLY: &[u8] =
        br#"<plist version="1.0"><dict><key>failureType</key><string>5002</string></dict></plist>"#;

    /// What issue #17 hit: a 301 with an HTML body and no `Location`.
    const EDGE_HTML: &[u8] = b"<html>\r\n<head><title>301 Moved Permanently</title></head>\r\n<body>\r\n<center><h1>301 Moved Permanently</h1></center>\r\n<hr><center>Apple</center>\r\n</body>\r\n</html>\r\n";

    #[test]
    fn resends_a_bare_redirect() {
        assert!(is_dropped_request(
            StatusCode::MOVED_PERMANENTLY,
            false,
            EDGE_HTML
        ));
    }

    #[test]
    fn keeps_a_pod_redirect_for_the_caller_to_follow() {
        assert!(!is_dropped_request(StatusCode::FOUND, true, b""));
    }

    #[test]
    fn resends_contentless_and_not_found_replies() {
        assert!(is_dropped_request(StatusCode::NO_CONTENT, false, b""));
        assert!(is_dropped_request(StatusCode::NOT_FOUND, false, EDGE_HTML));
    }

    #[test]
    fn resends_server_errors() {
        for status in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert!(is_dropped_request(status, false, b""), "{status}");
        }
    }

    /// Apple reports real store failures under a 5xx, so a plist body settles
    /// the request whatever the status says.
    #[test]
    fn keeps_a_store_reply_under_any_status() {
        assert!(!is_dropped_request(
            StatusCode::INTERNAL_SERVER_ERROR,
            false,
            STORE_REPLY
        ));
        assert!(!is_dropped_request(StatusCode::OK, false, STORE_REPLY));
    }

    /// The zero-length 403 is the application tier's verdict on a missing
    /// signature; resending it would only bury the diagnosis.
    #[test]
    fn keeps_the_unsigned_rejection() {
        assert!(!is_dropped_request(StatusCode::FORBIDDEN, false, b""));
    }

    /// Retrying a rate limit is exactly what a rate limit is asking us not to do.
    #[test]
    fn keeps_rate_limits() {
        assert!(!is_dropped_request(
            StatusCode::TOO_MANY_REQUESTS,
            false,
            b""
        ));
    }
}
