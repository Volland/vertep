//! Credentials, presentations and their metadata, independent of format.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use vertep_federation::resolver::key::did_key;
use vertep_federation::{Did, DidResolver, KeyManager, PublicKey};

use crate::error::WalletError;
use crate::history::{Claims, CredTerm};

/// A credential as issued: its format IRI and its serialisation (JSON text or a compact
/// string).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// The format IRI.
    pub format: String,
    /// The serialisation.
    pub raw: String,
}

impl Credential {
    /// `sha256:<hex>` of the serialisation: the credential's id in metadata and histories.
    pub fn digest(&self) -> String {
        digest(self.raw.as_bytes())
    }
}

/// `sha256:<hex>` of `bytes`.
pub fn digest(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Where a credential's status is published (W3C Bitstring Status List).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRef {
    /// The status list credential's URL.
    pub list: String,
    /// The entry's index.
    pub index: u64,
    /// `revocation` or `suspension`.
    pub purpose: String,
}

/// What the graph and the journal may know about a credential: no claim values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialMeta {
    /// `sha256:<hex>` of the credential.
    pub digest: String,
    /// The format IRI.
    pub format: String,
    /// Credential types (VC types or the SD-JWT `vct`).
    pub types: Vec<String>,
    /// The issuer DID.
    pub issuer: String,
    /// The issuer's verification method.
    pub issuer_key: String,
    /// The holder key the credential is bound to, as a `did:key`.
    pub holder_key: String,
    /// Valid from (milliseconds since the epoch).
    pub valid_from: Option<i64>,
    /// Valid until (milliseconds since the epoch).
    pub valid_until: Option<i64>,
    /// The status entry.
    pub status: Option<StatusRef>,
    /// Claim names.
    pub claims: Claims,
    /// Whether the format can disclose a subset of the claims.
    pub selective: bool,
}

impl CredentialMeta {
    /// The credential's term in exchange histories.
    pub fn term(&self) -> CredTerm {
        CredTerm {
            issuer: self.issuer.clone(),
            ikey: self.issuer_key.clone(),
            id: self.digest.clone(),
            key: self.holder_key.clone(),
            claims: self.claims.clone(),
        }
    }

    /// Whether `now` (milliseconds) is inside the validity period.
    pub fn valid_at(&self, now: i64) -> bool {
        self.valid_from.is_none_or(|f| now >= f) && self.valid_until.is_none_or(|u| now < u)
    }
}

/// A key the key manager holds, with the identifiers others see it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigningKey {
    /// The key's name in the key manager.
    pub key_name: String,
    /// The verification method id (a DID URL).
    pub kid: String,
    /// The controller DID.
    pub did: String,
    /// The public key.
    pub public: PublicKey,
}

impl SigningKey {
    /// The key `key_name` of `km`, identified as `did:key` (controller and method).
    ///
    /// # Errors
    ///
    /// [`WalletError::Key`] when the key manager does not hold it.
    pub fn did_key(km: &dyn KeyManager, key_name: &str) -> Result<Self, WalletError> {
        let public = km.public_key(key_name)?;
        let did = did_key(&public);
        let fragment = public.multikey();
        Ok(SigningKey {
            key_name: key_name.to_owned(),
            kid: format!("{did}#{fragment}"),
            did,
            public,
        })
    }

    /// The `did:key` of the public key (holder keys are identified this way).
    pub fn did_key_id(&self) -> String {
        did_key(&self.public)
    }
}

/// What to issue.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueRequest {
    /// Credential types, beyond `VerifiableCredential` (or the `vct` for SD-JWT VC).
    pub types: Vec<String>,
    /// Claims of the subject.
    pub claims: Map<String, Value>,
    /// The holder key to bind the credential to.
    pub holder_key: PublicKey,
    /// Valid from (milliseconds).
    pub valid_from: i64,
    /// Valid until (milliseconds).
    pub valid_until: Option<i64>,
    /// The status entry.
    pub status: Option<StatusRef>,
}

/// A presentation as sent: its format IRI and its serialisation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presentation {
    /// The format IRI.
    pub format: String,
    /// The serialisation.
    pub raw: String,
}

/// A request for a presentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationRequest {
    /// The verifier (the audience a presentation must name).
    pub verifier: String,
    /// The verifier's nonce.
    pub nonce: String,
    /// Requested claim names.
    pub claims: Claims,
    /// Acceptable credential types (empty: any).
    pub types: Vec<String>,
    /// Why the verifier asks.
    pub purpose: Option<String>,
}

/// A presentation whose signatures verified (status and trust are checked by the wallet).
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedPresentation {
    /// The presented credential's metadata.
    pub credential: CredentialMeta,
    /// The disclosed claims, with values.
    pub disclosed: Map<String, Value>,
    /// The key that signed the presentation, as a `did:key`.
    pub signer: String,
    /// The audience the presentation names.
    pub audience: String,
    /// The nonce the presentation answers.
    pub nonce: String,
}

impl VerifiedPresentation {
    /// The disclosed claim names.
    pub fn disclosed_names(&self) -> Claims {
        self.disclosed.keys().cloned().collect()
    }
}

/// Finds public keys by verification method id (a DID URL) or `did:key`.
pub trait KeyLookup: Send + Sync {
    /// The key of `method`, which must belong to `controller` when given.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when it cannot be found.
    fn key(&self, method: &str) -> Result<PublicKey, WalletError>;
}

/// Key lookup through a DID resolver; `did:key` identifiers resolve offline.
#[derive(Clone)]
pub struct ResolverKeys(pub Arc<dyn DidResolver>);

impl std::fmt::Debug for ResolverKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ResolverKeys")
    }
}

impl KeyLookup for ResolverKeys {
    fn key(&self, method: &str) -> Result<PublicKey, WalletError> {
        if let Some(rest) = method.strip_prefix("did:key:") {
            let mk = rest.split('#').next().unwrap_or(rest);
            return PublicKey::from_multikey(mk).map_err(WalletError::Key);
        }
        let did = Did::from_did_url(method).map_err(|e| WalletError::Malformed(e.to_string()))?;
        let doc = self
            .0
            .resolve(&did)
            .map_err(|e| WalletError::Malformed(format!("cannot resolve {did}: {e}")))?
            .document;
        doc.verification_method
            .iter()
            .find(|m| m.id == method)
            .map(|m| m.public_key.clone())
            .ok_or_else(|| WalletError::BadSignature(format!("{method} is not a key of {did}")))
    }
}

/// Key lookup that knows only `did:key` identifiers.
#[derive(Clone, Copy, Debug, Default)]
pub struct DidKeyOnly;

impl KeyLookup for DidKeyOnly {
    fn key(&self, method: &str) -> Result<PublicKey, WalletError> {
        let rest = method
            .strip_prefix("did:key:")
            .ok_or_else(|| WalletError::Malformed(format!("{method} is not a did:key")))?;
        let mk = rest.split('#').next().unwrap_or(rest);
        PublicKey::from_multikey(mk).map_err(WalletError::Key)
    }
}

/// The DID a verification method id belongs to.
pub fn controller_of(method: &str) -> &str {
    method.split('#').next().unwrap_or(method)
}
