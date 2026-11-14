//! JWS compact serialisation with EdDSA (Ed25519) through the key manager, and base64url.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use vertep_federation::{KeyManager, PublicKey};

use crate::error::WalletError;

/// base64url without padding.
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64url without padding.
///
/// # Errors
///
/// [`WalletError::Malformed`] for invalid input.
pub fn unb64(s: &str) -> Result<Vec<u8>, WalletError> {
    URL_SAFE_NO_PAD
        .decode(s.trim_end_matches('='))
        .map_err(|e| WalletError::Malformed(format!("base64url: {e}")))
}

/// A compact JWS of `payload` with `header` (whose `alg` is set to `EdDSA`), signed by the key
/// `key_name`.
///
/// # Errors
///
/// [`WalletError::Key`] when the key manager cannot sign.
pub fn sign(
    km: &dyn KeyManager,
    key_name: &str,
    header: &Value,
    payload: &Value,
) -> Result<String, WalletError> {
    let mut h = header.clone();
    if let Some(o) = h.as_object_mut() {
        o.insert("alg".into(), Value::from("EdDSA"));
    }
    let signing_input = format!(
        "{}.{}",
        b64(&serde_json::to_vec(&h).map_err(|e| WalletError::Malformed(e.to_string()))?),
        b64(&serde_json::to_vec(payload).map_err(|e| WalletError::Malformed(e.to_string()))?)
    );
    let sig = km.sign(key_name, signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", b64(&sig)))
}

/// A decoded compact JWS: header, payload and what was signed.
#[derive(Clone, Debug, PartialEq)]
pub struct Jws {
    /// The protected header.
    pub header: Value,
    /// The payload.
    pub payload: Value,
    signing_input: String,
    signature: Vec<u8>,
}

impl Jws {
    /// Parses a compact JWS (no signature check).
    ///
    /// # Errors
    ///
    /// [`WalletError::Malformed`] when it is not three base64url JSON parts.
    pub fn parse(compact: &str) -> Result<Self, WalletError> {
        let mut parts = compact.split('.');
        let (Some(h), Some(p), Some(s), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(WalletError::Malformed("not a compact JWS".into()));
        };
        let header: Value = serde_json::from_slice(&unb64(h)?)
            .map_err(|e| WalletError::Malformed(format!("JWS header: {e}")))?;
        let payload: Value = serde_json::from_slice(&unb64(p)?)
            .map_err(|e| WalletError::Malformed(format!("JWS payload: {e}")))?;
        if header.get("alg").and_then(Value::as_str) != Some("EdDSA") {
            return Err(WalletError::Malformed("JWS alg is not EdDSA".into()));
        }
        Ok(Jws {
            header,
            payload,
            signing_input: format!("{h}.{p}"),
            signature: unb64(s)?,
        })
    }

    /// Verifies the signature with `key`.
    ///
    /// # Errors
    ///
    /// [`WalletError::BadSignature`] when it does not verify.
    pub fn verify(&self, key: &PublicKey) -> Result<(), WalletError> {
        if key.verify(self.signing_input.as_bytes(), &self.signature) {
            Ok(())
        } else {
            Err(WalletError::BadSignature("JWS signature mismatch".into()))
        }
    }
}
