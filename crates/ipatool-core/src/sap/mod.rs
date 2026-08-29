//! Apple's SAP request signing.
//!
//! Since 2026-08 Apple's bag lists `MZFinance/authenticate` under
//! `sign-sap-request`, meaning the sign-in POST is only accepted when it
//! carries an `X-Apple-ActionSignature` header. Unsigned requests are rejected
//! by the application tier with a zero-length 403 regardless of credentials,
//! body, user agent or TLS stack.
//!
//! Producing that signature requires a session established with Apple through
//! a three-step handshake:
//!
//! 1. `GET sign-sap-setup-cert` for Apple's certificate;
//! 2. feed it to a local state machine, which emits a setup request;
//! 3. `POST` that request to `sign-sap-setup` and feed the reply back.
//!
//! Steps 1 and 3 are plain HTTP and live in [`setup`]. Step 2 is Apple's own
//! `CommerceKit`/`CoreFP` code, which [`machine`] runs under a CPU emulator —
//! the binaries are fetched and verified by [`assets`], the emulator by
//! [`unicorn`]. See issue #15 for why this is necessary.

pub mod assets;
pub mod machine;
pub mod macho;
pub mod setup;
pub mod unicorn;

use std::sync::Arc;

use url::Url;

use crate::client::AppleClient;
use crate::error::ClientError;

/// The only SAP protocol revision this client knows how to speak. Apple's bag
/// currently advertises `sign-sap-version = 200`.
pub const SUPPORTED_VERSION: u32 = 200;

/// Returns the lowercase hex SHA-256 of `data`.
///
/// Both the Apple assets and the Unicorn library are executed, so both are
/// checked against pinned digests before use.
pub(crate) fn hex_digest(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone)]
pub struct SapConfig {
    pub setup_url: Url,
    pub cert_url: Url,
    pub version: u32,
}

impl SapConfig {
    pub fn validate(&self) -> Result<(), ClientError> {
        if self.version != SUPPORTED_VERSION {
            return Err(ClientError::Sap(format!(
                "unsupported SAP version {} (expected {SUPPORTED_VERSION})",
                self.version
            )));
        }

        for (label, url) in [("setup", &self.setup_url), ("certificate", &self.cert_url)] {
            if url.scheme() != "https" || !url.has_host() {
                return Err(ClientError::Sap(format!(
                    "SAP {label} endpoint must be an absolute HTTPS URL, got {url}"
                )));
            }
        }

        Ok(())
    }
}

/// Signs an outgoing request body.
pub trait ActionSigner: Send + Sync {
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, ClientError>;
}

/// The local half of the handshake — Apple's `CommerceCore`/`CoreFP` state
/// machine. `context` is an opaque handle owned by the machine.
pub trait SapMachine: Send + Sync {
    fn initialize(&self, hardware_id: &[u8]) -> Result<u64, ClientError>;

    /// Advances the setup exchange. Returns the buffer to send to Apple (empty
    /// once setup is complete) and the resulting state: 1 = awaiting Apple's
    /// reply, 0 = established.
    fn exchange(
        &self,
        version: u32,
        hardware_id: &[u8],
        context: u64,
        input: &[u8],
    ) -> Result<(Vec<u8>, i32), ClientError>;

    fn sign(&self, context: u64, input: &[u8]) -> Result<Vec<u8>, ClientError>;

    fn teardown(&self, context: u64) -> Result<(), ClientError>;
}

pub struct Signer {
    machine: Arc<dyn SapMachine>,
    context: u64,
}

impl ActionSigner for Signer {
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, ClientError> {
        let signature = self.machine.sign(self.context, data)?;

        if signature.is_empty() {
            return Err(ClientError::Sap("signature is empty".into()));
        }

        Ok(signature)
    }
}

impl Drop for Signer {
    fn drop(&mut self) {
        if let Err(e) = self.machine.teardown(self.context) {
            tracing::warn!("failed to tear down SAP session: {e}");
        }
    }
}

/// Runs the full handshake and returns a signer bound to the resulting session.
pub async fn new_signer(
    client: &AppleClient,
    machine: Arc<dyn SapMachine>,
    config: &SapConfig,
    hardware_id: &[u8],
) -> Result<Signer, ClientError> {
    config.validate()?;

    if hardware_id.is_empty() || hardware_id.len() > 20 {
        return Err(ClientError::Sap(format!(
            "hardware ID must be 1-20 bytes, got {}",
            hardware_id.len()
        )));
    }

    let context = machine.initialize(hardware_id)?;

    // Tear the context down on any failure between here and the return.
    let signer = Signer {
        machine: Arc::clone(&machine),
        context,
    };

    let certificate = setup::fetch_certificate(client, &config.cert_url).await?;
    tracing::debug!(len = certificate.len(), "fetched SAP setup certificate");

    let (request, state) = machine.exchange(config.version, hardware_id, context, &certificate)?;

    if state != 1 {
        return Err(ClientError::Sap(format!(
            "SAP setup entered unexpected state {state}"
        )));
    }

    if request.is_empty() {
        return Err(ClientError::Sap("SAP setup message is empty".into()));
    }

    let reply = setup::exchange(client, &config.setup_url, &request).await?;

    let (_, state) = machine.exchange(config.version, hardware_id, context, &reply)?;

    if state != 0 {
        return Err(ClientError::Sap(format!(
            "SAP setup completed in unexpected state {state}"
        )));
    }

    tracing::debug!("SAP session established");

    Ok(signer)
}

/// Builds the emulated SAP backend and establishes a signer with it.
///
/// The Apple frameworks and the emulator are downloaded on first use and cached
/// under the client's cache directory, so only the first call is slow.
pub async fn new_default_signer(
    client: &AppleClient,
    config: &SapConfig,
    hardware_id: &[u8],
) -> Result<Signer, ClientError> {
    let cache_dir = client.cache_dir().to_path_buf();
    let bundle = assets::load(client, &cache_dir.join("apple-assets")).await?;
    let machine = machine::Machine::open(client, &cache_dir, &bundle).await?;

    new_signer(client, Arc::new(machine), config, hardware_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(setup: &str, cert: &str, version: u32) -> SapConfig {
        SapConfig {
            setup_url: Url::parse(setup).unwrap(),
            cert_url: Url::parse(cert).unwrap(),
            version,
        }
    }

    #[test]
    fn accepts_apples_current_configuration() {
        assert!(
            config(
                "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy",
                "https://s.mzstatic.com/sap/setupCert.plist",
                200,
            )
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn rejects_unsupported_version() {
        assert!(
            config(
                "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy",
                "https://s.mzstatic.com/sap/setupCert.plist",
                201,
            )
            .validate()
            .is_err()
        );
    }

    #[test]
    fn rejects_plaintext_endpoints() {
        assert!(
            config(
                "http://fpinit.itunes.apple.com/v1/signSapSetup/legacy",
                "https://s.mzstatic.com/sap/setupCert.plist",
                200,
            )
            .validate()
            .is_err()
        );
    }

    /// Runs the whole thing against Apple: downloads and verifies the
    /// frameworks and the emulator, boots the guest, performs the two-step
    /// handshake with `fpinit.itunes.apple.com`, and signs a request body.
    ///
    /// `cargo test -p ipatool-core -- --ignored live_sap`
    #[tokio::test]
    #[ignore = "performs a real SAP handshake with Apple"]
    async fn live_sap_handshake_produces_a_signature() {
        let client = crate::client::AppleClient::for_tests();

        let bag = crate::api::bag::fetch_bag(&client)
            .await
            .expect("fetch bag");

        let signer = new_default_signer(&client, &bag.sap, client.hardware_id())
            .await
            .expect("establish SAP session");

        let signature = signer.sign(b"<plist>body</plist>").expect("sign");

        assert!(!signature.is_empty());

        // Signing twice must keep working: the session outlives one call, and
        // the guest heap has to survive the allocation churn.
        let again = signer.sign(b"<plist>other</plist>").expect("sign again");

        assert!(!again.is_empty());
        assert_ne!(signature, again, "signature does not depend on the body");
    }
}
