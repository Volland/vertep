//! The signed message envelope.
//!
//! A message is a [`Body`] signed by the sender's DID key before any transport seals it. The
//! signature is over the JCS (RFC 8785) bytes of the body, so any holder of the plaintext can
//! check it without the transport, and the [`MessageHash`] covers the body, the key id and the
//! signature. Signing and verification enter through [`Signer`] and [`Verifier`]; the crate
//! holds no keys.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// The reserved message kinds of the control messages every protocol shares.
pub mod control {
    /// Opens an instance; signed by the initiator.
    pub const GENESIS: &str = "kolyada:genesis";
    /// A participant's acceptance of a role.
    pub const JOIN: &str = "kolyada:join";
    /// A participant's refusal of a role.
    pub const DECLINE: &str = "kolyada:decline";
    /// A signed notice that a message was refused.
    pub const VIOLATION: &str = "kolyada:violation";
    /// Marks a finished instance, for the parent that opened it.
    pub const COMPLETE: &str = "kolyada:complete";
}

/// The hash of a message: lowercase hex SHA-256 of the JCS bytes of the whole message.
pub type MessageHash = String;

/// What a signature covers.
// @tg: implements:: [[openspec:protocol-spaces#Pairwise Sealed Fan-Out]] {confidence: partial, note: "the recipient set; sealing and mailboxes live in vertep-federation"}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    /// The protocol identity.
    pub protocol: String,
    /// The instance id, the genesis hash; absent in the genesis itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// The sender's DID.
    pub sender: String,
    /// The role the sender plays.
    pub role: String,
    /// The sender's sequence number in the instance, rising with each message it signs.
    pub seq: u64,
    /// The hashes of the messages this one depends on, sorted.
    pub deps: Vec<MessageHash>,
    /// The roles the message is sealed to, sorted.
    pub recipients: Vec<String>,
    /// The message type, or a [`control`] kind.
    #[serde(rename = "type")]
    pub kind: String,
    /// The payload.
    pub payload: Value,
}

/// A signed message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// The signed body.
    pub body: Body,
    /// The id of the verification method that signed it.
    pub key: String,
    /// The Ed25519 signature over the JCS bytes of the body, base64url without padding.
    pub signature: String,
}

/// Signing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignError(pub String);

/// A signature did not verify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError(pub String);

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "signing failed: {}", self.0)
    }
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "signature refused: {}", self.0)
    }
}

impl std::error::Error for SignError {}
impl std::error::Error for VerifyError {}

/// Signs on behalf of one DID. `vertep-federation` implements it over the key manager.
pub trait Signer {
    /// The DID signed for.
    fn did(&self) -> &str;
    /// The verification method id the signature is made under.
    fn key_id(&self) -> &str;
    /// An Ed25519 signature over `data`.
    ///
    /// # Errors
    ///
    /// [`SignError`] when the key cannot sign.
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignError>;
}

/// Checks that a key of a DID made a signature. `vertep-federation` implements it over its DID
/// resolver.
pub trait Verifier {
    /// Succeeds when `signature` is a valid signature over `data` by the key `key_id` of `did`.
    ///
    /// # Errors
    ///
    /// [`VerifyError`] when the key is unknown to the DID or the signature does not verify.
    fn verify(
        &self,
        did: &str,
        key_id: &str,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), VerifyError>;
}

impl Body {
    /// Sorts and de-duplicates `deps` and `recipients`, so equal bodies serialize equally.
    pub fn normalize(&mut self) {
        self.deps.sort();
        self.deps.dedup();
        self.recipients.sort();
        self.recipients.dedup();
    }

    /// The JCS bytes a signature covers.
    #[must_use]
    pub fn signing_bytes(&self) -> Vec<u8> {
        // A body is plain JSON, so canonicalization cannot fail.
        serde_json_canonicalizer::to_vec(self).unwrap_or_default()
    }
}

impl Message {
    /// Signs a body.
    ///
    /// # Errors
    ///
    /// [`SignError`] when the signer refuses, or signs for another DID than `body.sender`.
    // @tg: implements:: [[openspec:protocol-conformance#Signed Message Envelope]]
    pub fn sign(mut body: Body, signer: &dyn Signer) -> Result<Self, SignError> {
        if signer.did() != body.sender {
            return Err(SignError(format!(
                "signer is {}, body sender is {}",
                signer.did(),
                body.sender
            )));
        }
        body.normalize();
        let signature = URL_SAFE_NO_PAD.encode(signer.sign(&body.signing_bytes())?);
        Ok(Self {
            body,
            key: signer.key_id().to_owned(),
            signature,
        })
    }

    /// Checks the signature against the sender's DID.
    ///
    /// # Errors
    ///
    /// [`VerifyError`] when the signature is malformed or does not verify.
    // @tg: implements:: [[openspec:protocol-conformance#Signed Message Envelope]]
    pub fn verify(&self, verifier: &dyn Verifier) -> Result<(), VerifyError> {
        let sig = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|e| VerifyError(e.to_string()))?;
        verifier.verify(
            &self.body.sender,
            &self.key,
            &self.body.signing_bytes(),
            &sig,
        )
    }

    /// The hash of the message: body, key id and signature.
    #[must_use]
    pub fn hash(&self) -> MessageHash {
        let bytes = serde_json_canonicalizer::to_vec(self).unwrap_or_default();
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The message as JSON text.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Reads a message from JSON text.
    ///
    /// # Errors
    ///
    /// A message of the wrong shape.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}
