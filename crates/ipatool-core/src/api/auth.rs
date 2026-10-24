use reqwest::{StatusCode, header::HeaderMap};
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

use crate::client::AppleClient;
use crate::client::plist_xml::{is_html_response, parse_plist_response};
use crate::error::{ClientError, StoreError};
use crate::model::Account;
use crate::sap::ActionSigner;

const MAX_ATTEMPTS: u32 = 4;
const MAX_REDIRECTS: u32 = 5;
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_AUTH_SENDS: u32 = 3;
const AUTH_RESEND_DELAY: Duration = Duration::from_millis(250);

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

        tracing::debug!(
            attempt,
            host = current_url.host_str(),
            signed = signer.is_some(),
            "sending auth request"
        );

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

        let resp = send_auth_request(request.body(body_bytes)).await?;

        let status = resp.status;
        tracing::debug!(%status, "auth response status");

        if status.is_redirection()
            && let Some(location) = resp.headers.get("location")
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
            .headers
            .get("x-set-apple-store-front")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let pod = resp
            .headers
            .get("pod")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let resp_body = &resp.body;
        tracing::debug!(len = resp_body.len(), "auth response body length");

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

        let dict = resp.parse()?;

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

struct AuthResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: bytes::Bytes,
}

impl AuthResponse {
    fn parse(&self) -> Result<HashMap<String, plist::Value>, ClientError> {
        parse_plist_response(&self.body).map_err(|_| {
            // Never include raw headers, URLs, body snippets or parser errors:
            // even a failed response can contain credentials or session tokens.
            let content_type = self.headers.get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or_default().trim());
            let media_type = match content_type {
                Some(v) if v.eq_ignore_ascii_case("text/html") => "text/html",
                Some(v) if v.eq_ignore_ascii_case("application/xhtml+xml") => "application/xhtml+xml",
                Some(v) if v.eq_ignore_ascii_case("text/xml") => "text/xml",
                Some(v) if v.eq_ignore_ascii_case("application/x-apple-plist") => "application/x-apple-plist",
                Some(_) => "other",
                None => "missing",
            };
            let kind = if is_html_response(&self.body) { "HTML" } else { "an invalid property list" };
            ClientError::UnexpectedResponse(format!(
                "Apple sign-in returned {kind} (HTTP {}, content-type {media_type}, {} bytes). \
                 Apple or a network intermediary may have returned an error page; \
                 check the network, VPN or proxy before trying again. Response contents omitted for privacy",
                self.status.as_u16(), self.body.len()
            ))
        })
    }
}

async fn send_auth_request(request: reqwest::RequestBuilder) -> Result<AuthResponse, ClientError> {
    let mut sends = 0;
    loop {
        // Clone after signing: a resend must preserve both the signed bytes
        // and Apple's attempt counter. Production auth bodies are in-memory.
        let resp = request
            .try_clone()
            .ok_or_else(|| {
                ClientError::UnexpectedResponse("auth request body cannot be replayed".into())
            })?
            .send()
            .await?;
        sends += 1;
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.bytes().await?;
        let response = AuthResponse {
            status,
            headers,
            body,
        };

        // A valid store error is authoritative even under HTTP 5xx. Never
        // resend a 403 or 429, or consume a redirect with a real Location.
        let transient = status == StatusCode::NO_CONTENT
            || status == StatusCode::NOT_FOUND
            || status.is_server_error()
            || (status.is_redirection() && !response.headers.contains_key("location"));
        let has_store_reply =
            parse_plist_response::<HashMap<String, plist::Value>>(&response.body).is_ok();
        if !transient || has_store_reply || sends == MAX_AUTH_SENDS {
            return Ok(response);
        }
        tracing::warn!(%status, sends, "resending sign-in after a non-plist response");
        tokio::time::sleep(AUTH_RESEND_DELAY).await;
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    const PLIST: &[u8] =
        b"<plist><dict><key>failureType</key><string>-5000</string></dict></plist>";
    const HTML: &[u8] = b"<html>\n<body>PRIVATE_RESPONSE_TOKEN</body></html>";

    // Exercise real HTTP sends without a mock dependency or Apple credentials.
    fn server(
        replies: Vec<(&'static str, &'static str, &'static [u8])>,
    ) -> (Url, thread::JoinHandle<Vec<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let handle = thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut requests = Vec::new();
            for (status, headers, body) in replies {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= deadline {
                                return requests;
                            }
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                let length: usize = String::from_utf8_lossy(&request)
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|value| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                let offset = request.len();
                request.resize(offset + length, 0);
                stream.read_exact(&mut request[offset..]).unwrap();
                requests.push(request);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
            requests
        });
        (url, handle)
    }

    fn request(url: Url) -> reqwest::RequestBuilder {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
            .post(url)
            .timeout(Duration::from_secs(3))
            .header(ACTION_SIGNATURE_HEADER, "SYNTHETIC_SIGNATURE")
            .body("SYNTHETIC_SIGNED_BODY")
    }

    #[tokio::test]
    async fn retries_locationless_html_and_empty_204_with_identical_signed_bytes() {
        let (url, handle) = server(vec![
            ("301 Moved Permanently", "Content-Type: text/html\r\n", HTML),
            ("204 No Content", "", b""),
            ("200 OK", "Content-Type: text/xml\r\n", PLIST),
        ]);
        let response = send_auth_request(request(url)).await.unwrap();
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.windows(2).all(|pair| pair[0] == pair[1]));
        assert!(matches!(
            StoreError::from_plist_dict(&response.parse().unwrap()),
            Some(StoreError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn html_404_is_resent_before_accepting_a_store_reply() {
        let (url, handle) = server(vec![
            ("404 Not Found", "Content-Type: text/html\r\n", HTML),
            ("200 OK", "Content-Type: text/xml\r\n", PLIST),
        ]);
        let response = send_auth_request(request(url)).await.unwrap();
        assert_eq!(handle.join().unwrap().len(), 2);
        assert!(response.parse().is_ok());
    }

    #[tokio::test]
    async fn exhausted_html_retries_have_safe_http_diagnostics() {
        let (url, handle) = server(vec![
            (
                "500 Internal Server Error",
                "Content-Type: text/html; secret=HEADER_TOKEN\r\n",
                HTML
            );
            3
        ]);
        let response = send_auth_request(request(url)).await.unwrap();
        assert_eq!(handle.join().unwrap().len(), 3);
        let err = response.parse().unwrap_err();
        for message in [err.to_string(), format!("{err:?}")] {
            assert!(message.contains("HTML"));
            assert!(message.contains("HTTP 500"));
            assert!(message.contains("content-type text/html"));
            assert!(!message.contains("PRIVATE_RESPONSE_TOKEN"));
            assert!(!message.contains("HEADER_TOKEN"));
        }
    }

    #[tokio::test]
    async fn store_plists_redirects_forbidden_and_rate_limits_are_not_resent() {
        for (status, headers, body) in [
            (
                "500 Internal Server Error",
                "Content-Type: text/xml\r\n",
                PLIST,
            ),
            (
                "302 Found",
                "Location: https://p14-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate\r\n",
                HTML,
            ),
            ("403 Forbidden", "", b"".as_slice()),
            ("403 Forbidden", "Content-Type: text/html\r\n", HTML),
            ("429 Too Many Requests", "Content-Type: text/html\r\n", HTML),
            ("200 OK", "Content-Type: text/html\r\n", HTML),
            (
                "500 Internal Server Error",
                "Content-Type: text/html\r\n",
                PLIST,
            ),
        ] {
            let (url, handle) = server(vec![(status, headers, body)]);
            let response = send_auth_request(request(url)).await.unwrap();
            assert_eq!(handle.join().unwrap().len(), 1);
            assert_eq!(response.body.as_ref(), body);
            if body == PLIST {
                assert!(response.parse().is_ok());
            }
        }
    }

    #[test]
    fn malformed_responses_do_not_leak_parser_values_or_unknown_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "PRIVATE_HEADER_TOKEN".parse().unwrap());
        let response = AuthResponse {
            status: StatusCode::OK,
            headers,
            body: bytes::Bytes::from_static(b"<plist><string>PRIVATE_BODY_TOKEN</string></plist>"),
        };
        let error = response.parse().unwrap_err();
        assert!(!format!("{error:?} {error}").contains("PRIVATE_"));
        assert!(error.to_string().contains("HTTP 200"));
    }
}
