//! The five plugin interfaces of the wallet and the registry that maps IRIs to them
//! (`agent-wallet` requirement `Pluggable Wallet Interfaces`, design D3).
//!
//! The wallet hard-codes no protocol, format, store, consent policy or status mechanism. An
//! application adds plugins under its own IRIs, or replaces a built-in by registering under the
//! built-in's IRI; an IRI nobody registered fails with [`WalletError::UnknownPlugin`].

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde_json::{Map, Value};

use crate::credential::{
    Credential, CredentialMeta, IssueRequest, KeyLookup, Presentation, SigningKey, StatusRef,
    VerifiedPresentation,
};
use crate::error::WalletError;
use crate::history::Claims;
use crate::message::ExchangeMessage;
use crate::vocab::Role;

/// A credential format: issue, verify, derive a presentation, check holder binding.
pub trait CredentialFormat: Send + Sync {
    /// The format IRI.
    fn iri(&self) -> &str;

    /// Issues `request`, signed by `issuer`.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when signing fails.
    fn issue(
        &self,
        km: &dyn vertep_federation::KeyManager,
        issuer: &SigningKey,
        request: &IssueRequest,
    ) -> Result<Credential, WalletError>;

    /// Verifies the issuer's signature and returns the credential's metadata. Validity
    /// period, status and trust are the caller's checks.
    ///
    /// # Errors
    ///
    /// [`WalletError`] naming the failed check.
    fn verify(
        &self,
        credential: &Credential,
        keys: &dyn KeyLookup,
    ) -> Result<CredentialMeta, WalletError>;

    /// The credential's claims with values (the holder's own view).
    ///
    /// # Errors
    ///
    /// [`WalletError::Malformed`] when the credential cannot be read.
    fn claims(&self, credential: &Credential) -> Result<Map<String, Value>, WalletError>;

    /// A presentation of `credential` disclosing `disclose`, for `audience` and `nonce`, signed
    /// by the holder key the credential is bound to.
    ///
    /// # Errors
    ///
    /// [`WalletError`] when the format cannot disclose exactly `disclose` or signing fails.
    fn present(
        &self,
        km: &dyn vertep_federation::KeyManager,
        credential: &Credential,
        holder: &SigningKey,
        audience: &str,
        nonce: &str,
        disclose: &Claims,
    ) -> Result<Presentation, WalletError>;

    /// Verifies a presentation's signatures: the issuer's over the credential and the
    /// holder's over the presentation (holder binding).
    ///
    /// # Errors
    ///
    /// [`WalletError`] naming the failed check.
    fn verify_presentation(
        &self,
        presentation: &Presentation,
        keys: &dyn KeyLookup,
    ) -> Result<VerifiedPresentation, WalletError>;
}

/// A store of credentials. The built-in [`crate::store::SealedStore`] encrypts every record
/// at rest; any store must keep claim values out of the graph.
pub trait CredentialStore: Send + Sync {
    /// Stores `credential` with its metadata.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`] when it cannot be stored.
    fn put(&self, meta: &CredentialMeta, credential: &Credential) -> Result<(), WalletError>;

    /// The credential with `digest`, checked against its digest.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownCredential`] or [`WalletError::Integrity`].
    fn get(&self, digest: &str) -> Result<(CredentialMeta, Credential), WalletError>;

    /// Every stored credential's metadata.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`] when the store cannot be read.
    fn list(&self) -> Result<Vec<CredentialMeta>, WalletError>;

    /// Erases the credential with `digest`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`] when it cannot be erased.
    fn erase(&self, digest: &str) -> Result<(), WalletError>;
}

/// One disclosure a holder may make: a credential and the claims to disclose from it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct DisclosureOption {
    /// The credential's digest.
    pub credential: String,
    /// The claims disclosed.
    pub claims: Claims,
}

/// What a consent policy decides on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsentRequest {
    /// The verifier.
    pub verifier: String,
    /// The verifier's nonce.
    pub nonce: String,
    /// Requested claim names.
    pub requested: Claims,
    /// Acceptable credential types (empty: any).
    pub types: Vec<String>,
    /// Why the verifier asks.
    pub purpose: Option<String>,
    /// The live credentials (unexpired, unrevoked, undeleted).
    pub held: Vec<CredentialMeta>,
}

/// A consent policy: the disclosures allowed for a request. The holder's decide state offers
/// exactly these, plus refusing.
pub trait ConsentPolicy: Send + Sync {
    /// The allowed disclosures, each within the request and the policy.
    fn options(&self, request: &ConsentRequest) -> Vec<DisclosureOption>;
}

/// A status resolver: whether a credential's status entry is set.
pub trait StatusResolver: Send + Sync {
    /// Whether the entry `status` of a credential from `issuer` is set (revoked or suspended)
    /// at `now` (milliseconds). The status list must be issued by `issuer`.
    ///
    /// # Errors
    ///
    /// [`WalletError::StatusUnavailable`] when the list cannot be fetched or verified.
    fn is_set(&self, status: &StatusRef, issuer: &str, now: i64) -> Result<bool, WalletError>;
}

/// An exchange protocol: role charts (by default the generic ones of
/// [`crate::exchange::charts`]), and a codec between wire bytes and [`ExchangeMessage`]s.
pub trait ExchangeProtocol: Send + Sync {
    /// The protocol IRI.
    fn iri(&self) -> &str;

    /// The SCXML of the role chart for `role`.
    fn role_chart(&self, role: Role) -> &'static str {
        crate::exchange::charts::generic(role)
    }

    /// Encodes `message` for the wire.
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] when it cannot be encoded.
    fn encode(&self, message: &ExchangeMessage) -> Result<Vec<u8>, WalletError>;

    /// The audience a verifier with DID `verifier` is named by in presentations (the DID
    /// itself unless the protocol uses another client identifier).
    fn audience(&self, verifier: &str) -> String {
        verifier.to_owned()
    }

    /// Decodes wire bytes. A message the protocol does not map onto the exchange vocabulary
    /// is [`WalletError::Unmapped`].
    ///
    /// # Errors
    ///
    /// [`WalletError::Unmapped`] or [`WalletError::Malformed`].
    fn decode(&self, bytes: &[u8]) -> Result<ExchangeMessage, WalletError>;
}

type Map2<T> = RwLock<BTreeMap<String, Arc<T>>>;

/// The registry of wallet plugins, by IRI.
#[derive(Default)]
pub struct WalletRegistry {
    protocols: Map2<dyn ExchangeProtocol>,
    formats: Map2<dyn CredentialFormat>,
    stores: Map2<dyn CredentialStore>,
    consent: Map2<dyn ConsentPolicy>,
    status: Map2<dyn StatusResolver>,
}

impl std::fmt::Debug for WalletRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletRegistry")
            .field("protocols", &self.protocol_iris())
            .field("formats", &self.format_iris())
            .finish_non_exhaustive()
    }
}

fn get<T: ?Sized>(m: &Map2<T>, kind: &'static str, iri: &str) -> Result<Arc<T>, WalletError> {
    m.read()
        .ok()
        .and_then(|m| m.get(iri).cloned())
        .ok_or_else(|| WalletError::UnknownPlugin {
            kind,
            iri: iri.to_owned(),
        })
}

fn put<T: ?Sized>(m: &Map2<T>, iri: &str, v: Arc<T>) {
    if let Ok(mut m) = m.write() {
        m.insert(iri.to_owned(), v);
    }
}

// @tg: implements:: [[openspec:agent-wallet#Pluggable Wallet Interfaces]]
impl WalletRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the built-in formats, the native protocol, the rule consent policy
    /// (refuse everything) and no store or status resolver (those need configuration).
    pub fn with_builtins() -> Self {
        let r = Self::new();
        r.register_format(Arc::new(crate::format::di::DataIntegrity::jcs()));
        r.register_format(Arc::new(crate::format::di::DataIntegrity::rdfc()));
        #[cfg(feature = "sd-jwt")]
        r.register_format(Arc::new(crate::format::sdjwt::SdJwtVc));
        r.register_protocol(Arc::new(crate::protocol::native::VertepExchange));
        r.register_protocol(Arc::new(crate::protocol::oid4vc::Oid4vci));
        r.register_protocol(Arc::new(crate::protocol::oid4vc::Oid4vp));
        r.register_consent(
            crate::vocab::CONSENT_RULES,
            Arc::new(crate::consent::RulePolicy::refuse_all()),
        );
        r
    }

    /// Registers (or replaces) an exchange protocol under its IRI.
    pub fn register_protocol(&self, p: Arc<dyn ExchangeProtocol>) {
        let iri = p.iri().to_owned();
        put(&self.protocols, &iri, p);
    }

    /// Registers (or replaces) a credential format under its IRI.
    pub fn register_format(&self, f: Arc<dyn CredentialFormat>) {
        let iri = f.iri().to_owned();
        put(&self.formats, &iri, f);
    }

    /// Registers (or replaces) a credential store under `iri`.
    pub fn register_store(&self, iri: &str, s: Arc<dyn CredentialStore>) {
        put(&self.stores, iri, s);
    }

    /// Registers (or replaces) a consent policy under `iri`.
    pub fn register_consent(&self, iri: &str, c: Arc<dyn ConsentPolicy>) {
        put(&self.consent, iri, c);
    }

    /// Registers (or replaces) a status resolver under `iri`.
    pub fn register_status(&self, iri: &str, s: Arc<dyn StatusResolver>) {
        put(&self.status, iri, s);
    }

    /// The protocol registered under `iri`.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownPlugin`].
    pub fn protocol(&self, iri: &str) -> Result<Arc<dyn ExchangeProtocol>, WalletError> {
        get(&self.protocols, "protocol", iri)
    }

    /// The format registered under `iri`.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownPlugin`].
    pub fn format(&self, iri: &str) -> Result<Arc<dyn CredentialFormat>, WalletError> {
        get(&self.formats, "format", iri)
    }

    /// The store registered under `iri`.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownPlugin`].
    pub fn store(&self, iri: &str) -> Result<Arc<dyn CredentialStore>, WalletError> {
        get(&self.stores, "store", iri)
    }

    /// The consent policy registered under `iri`.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownPlugin`].
    pub fn consent(&self, iri: &str) -> Result<Arc<dyn ConsentPolicy>, WalletError> {
        get(&self.consent, "consent", iri)
    }

    /// The status resolver registered under `iri`.
    ///
    /// # Errors
    ///
    /// [`WalletError::UnknownPlugin`].
    pub fn status(&self, iri: &str) -> Result<Arc<dyn StatusResolver>, WalletError> {
        get(&self.status, "status", iri)
    }

    /// The registered protocol IRIs.
    pub fn protocol_iris(&self) -> Vec<String> {
        self.protocols
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// The registered format IRIs.
    pub fn format_iris(&self) -> Vec<String> {
        self.formats
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}
