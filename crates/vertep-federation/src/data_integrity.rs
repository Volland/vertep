//! W3C Data Integrity proofs with the EdDSA cryptosuites, signed through a [`KeyManager`].
//!
//! The hash data of an EdDSA proof is SHA-256 of the canonical proof configuration followed by
//! SHA-256 of the canonical unsecured document, signed with Ed25519. The cryptosuites differ
//! only in the canonicalisation: `eddsa-jcs-2022` uses JCS (RFC 8785) and `eddsa-rdfc-2022`
//! RDF Dataset Canonicalization; this module takes the canonicalisation as a function so both
//! (and grant credentials) share one signing and verification path. The proof configuration
//! carries the document's `@context`, as the cryptosuites require.

use std::fmt;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::keys::{KeyError, KeyManager, PublicKey};

/// The JCS EdDSA cryptosuite.
pub const EDDSA_JCS_2022: &str = "eddsa-jcs-2022";
/// The RDF canonicalisation EdDSA cryptosuite.
pub const EDDSA_RDFC_2022: &str = "eddsa-rdfc-2022";

/// A canonicalisation: the bytes a document (or proof configuration) is hashed as.
pub type Canonicalize<'a> = &'a dyn Fn(&Value) -> Result<Vec<u8>, String>;

/// Why a proof could not be made or did not verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofError {
    /// The document or proof is not well formed.
    Malformed(String),
    /// The proof uses another type, cryptosuite or purpose than expected.
    Unsupported(String),
    /// Canonicalisation failed.
    Canonicalization(String),
    /// The signature does not verify.
    BadSignature,
    /// The key manager could not sign.
    Signing(KeyError),
}

impl fmt::Display for ProofError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProofError::Malformed(m) => write!(f, "malformed proof: {m}"),
            ProofError::Unsupported(m) => write!(f, "unsupported proof: {m}"),
            ProofError::Canonicalization(m) => write!(f, "canonicalisation failed: {m}"),
            ProofError::BadSignature => f.write_str("signature mismatch"),
            ProofError::Signing(e) => write!(f, "signing failed: {e}"),
        }
    }
}

impl std::error::Error for ProofError {}

/// The options of a proof to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofOptions {
    /// The cryptosuite (`eddsa-jcs-2022` or `eddsa-rdfc-2022`).
    pub cryptosuite: String,
    /// The verification method, a DID URL.
    pub verification_method: String,
    /// The proof purpose (`assertionMethod` for credentials, `authentication` for
    /// presentations).
    pub purpose: String,
    /// The creation time (RFC 3339).
    pub created: String,
    /// A challenge (a verifier's nonce), for presentations.
    pub challenge: Option<String>,
    /// A domain (a verifier's identifier), for presentations.
    pub domain: Option<String>,
}

/// JCS (RFC 8785), the `eddsa-jcs-2022` canonicalisation.
///
/// # Errors
///
/// A message when the value cannot be canonicalised.
pub fn jcs(v: &Value) -> Result<Vec<u8>, String> {
    serde_json_canonicalizer::to_vec(v).map_err(|e| e.to_string())
}

fn hash_data(
    unsecured: &Value,
    proof_config: &Value,
    canon: Canonicalize<'_>,
) -> Result<Vec<u8>, ProofError> {
    let mut out = Sha256::digest(canon(proof_config).map_err(ProofError::Canonicalization)?)
        .to_vec();
    out.extend_from_slice(&Sha256::digest(
        canon(unsecured).map_err(ProofError::Canonicalization)?,
    ));
    Ok(out)
}

/// `document` (without a `proof`) secured with a proof made by the key `key_name`.
///
/// # Errors
///
/// [`ProofError`] when the document is not an object, canonicalisation fails or the key manager
/// cannot sign.
pub fn sign(
    km: &dyn KeyManager,
    key_name: &str,
    document: &Value,
    options: &ProofOptions,
    canon: Canonicalize<'_>,
) -> Result<Value, ProofError> {
    let mut doc = document
        .as_object()
        .cloned()
        .ok_or_else(|| ProofError::Malformed("not a JSON object".into()))?;
    doc.remove("proof");
    let unsecured = Value::Object(doc.clone());
    let mut config = Map::new();
    if let Some(ctx) = doc.get("@context") {
        config.insert("@context".into(), ctx.clone());
    }
    config.insert("type".into(), json!("DataIntegrityProof"));
    config.insert("cryptosuite".into(), json!(options.cryptosuite));
    config.insert("created".into(), json!(options.created));
    config.insert(
        "verificationMethod".into(),
        json!(options.verification_method),
    );
    config.insert("proofPurpose".into(), json!(options.purpose));
    if let Some(c) = &options.challenge {
        config.insert("challenge".into(), json!(c));
    }
    if let Some(d) = &options.domain {
        config.insert("domain".into(), json!(d));
    }
    let config = Value::Object(config);
    let data = hash_data(&unsecured, &config, canon)?;
    let sig = km.sign(key_name, &data).map_err(ProofError::Signing)?;
    let mut proof = config;
    if let Some(p) = proof.as_object_mut() {
        p.remove("@context");
        p.insert(
            "proofValue".into(),
            json!(format!("z{}", bs58::encode(sig).into_string())),
        );
    }
    doc.insert("proof".into(), proof);
    Ok(Value::Object(doc))
}

/// The proof of a secured document, checked to be a Data Integrity proof of `cryptosuite`
/// with `purpose`; the signature is not checked (see [`verify`]).
///
/// # Errors
///
/// [`ProofError`] naming what is missing or different.
pub fn proof_of<'a>(
    document: &'a Value,
    cryptosuite: &str,
    purpose: &str,
) -> Result<&'a Map<String, Value>, ProofError> {
    let proof = document
        .get("proof")
        .and_then(Value::as_object)
        .ok_or_else(|| ProofError::Malformed("no proof".into()))?;
    if proof.get("type").and_then(Value::as_str) != Some("DataIntegrityProof") {
        return Err(ProofError::Unsupported("not a DataIntegrityProof".into()));
    }
    if proof.get("cryptosuite").and_then(Value::as_str) != Some(cryptosuite) {
        return Err(ProofError::Unsupported(format!("not {cryptosuite}")));
    }
    if proof.get("proofPurpose").and_then(Value::as_str) != Some(purpose) {
        return Err(ProofError::Unsupported(format!("purpose is not {purpose}")));
    }
    Ok(proof)
}

/// Verifies the proof of `document` against `key`: the signature over the hash data of the
/// document without its proof and the proof configuration.
///
/// # Errors
///
/// [`ProofError::BadSignature`] when it does not verify, and other variants when the proof is
/// malformed.
pub fn verify(
    document: &Value,
    key: &PublicKey,
    canon: Canonicalize<'_>,
) -> Result<(), ProofError> {
    let proof = document
        .get("proof")
        .and_then(Value::as_object)
        .ok_or_else(|| ProofError::Malformed("no proof".into()))?;
    let sig = proof
        .get("proofValue")
        .and_then(Value::as_str)
        .and_then(|p| p.strip_prefix('z'))
        .ok_or_else(|| ProofError::Malformed("no base58btc proofValue".into()))
        .and_then(|p| {
            bs58::decode(p)
                .into_vec()
                .map_err(|e| ProofError::Malformed(e.to_string()))
        })?;
    let mut unsecured = document.clone();
    if let Some(o) = unsecured.as_object_mut() {
        o.remove("proof");
    }
    let mut config = Map::new();
    if let Some(ctx) = document.get("@context") {
        config.insert("@context".into(), ctx.clone());
    }
    for (k, v) in proof {
        if k != "proofValue" {
            config.insert(k.clone(), v.clone());
        }
    }
    let data = hash_data(&unsecured, &Value::Object(config), canon)?;
    if key.verify(&data, &sig) {
        Ok(())
    } else {
        Err(ProofError::BadSignature)
    }
}
