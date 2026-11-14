//! Wallet errors. Every error names the check that failed.

use std::fmt;

use vertep_federation::KeyError;

/// Why a wallet operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalletError {
    /// An identity already has a wallet.
    Exists(String),
    /// No plugin is registered under this IRI.
    UnknownPlugin {
        /// The kind of plugin (`protocol`, `format`, `store`, `consent`, `status`).
        kind: &'static str,
        /// The IRI.
        iri: String,
    },
    /// No credential with this digest.
    UnknownCredential(String),
    /// A stored record does not match its digest, or does not decrypt.
    Integrity(String),
    /// A credential or presentation is malformed.
    Malformed(String),
    /// A signature or proof does not verify.
    BadSignature(String),
    /// The issuer is not trusted.
    UntrustedIssuer(String),
    /// The presentation is signed by another key than the credential is bound to.
    HolderBinding(String),
    /// The presentation is for another verifier (audience).
    WrongAudience(String),
    /// The nonce is unknown, reused or expired.
    Nonce(String),
    /// A disclosed claim was not requested.
    UnrequestedClaim(String),
    /// The credential is revoked.
    Revoked(String),
    /// The credential is outside its validity period.
    NotValidNow(String),
    /// Status could not be fetched or verified.
    StatusUnavailable(String),
    /// No credential satisfies the request.
    NoMatchingCredential(String),
    /// The consent step refused or chose outside its bounds.
    ConsentRefused(String),
    /// A message the protocol does not map onto the exchange vocabulary.
    Unmapped(String),
    /// A protocol or exchange error.
    Protocol(String),
    /// Storage failed.
    Storage(String),
    /// A key-manager operation failed.
    Key(KeyError),
}

impl fmt::Display for WalletError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalletError::Exists(w) => write!(f, "a wallet already exists: {w}"),
            WalletError::UnknownPlugin { kind, iri } => {
                write!(f, "no {kind} plugin is registered under {iri}")
            }
            WalletError::UnknownCredential(d) => write!(f, "no credential {d}"),
            WalletError::Integrity(m) => write!(f, "integrity check failed: {m}"),
            WalletError::Malformed(m) => write!(f, "malformed: {m}"),
            WalletError::BadSignature(m) => write!(f, "signature check failed: {m}"),
            WalletError::UntrustedIssuer(i) => write!(f, "untrusted issuer {i}"),
            WalletError::HolderBinding(m) => write!(f, "holder binding check failed: {m}"),
            WalletError::WrongAudience(a) => write!(f, "audience check failed: {a}"),
            WalletError::Nonce(m) => write!(f, "nonce check failed: {m}"),
            WalletError::UnrequestedClaim(c) => write!(f, "claim {c} was not requested"),
            WalletError::Revoked(c) => write!(f, "credential {c} is revoked"),
            WalletError::NotValidNow(c) => write!(f, "credential {c} is not valid now"),
            WalletError::StatusUnavailable(m) => write!(f, "status unavailable: {m}"),
            WalletError::NoMatchingCredential(m) => {
                write!(f, "no credential satisfies the request: {m}")
            }
            WalletError::ConsentRefused(m) => write!(f, "consent refused: {m}"),
            WalletError::Unmapped(m) => {
                write!(f, "message not mapped onto the exchange vocabulary: {m}")
            }
            WalletError::Protocol(m) => write!(f, "protocol error: {m}"),
            WalletError::Storage(m) => write!(f, "storage error: {m}"),
            WalletError::Key(e) => write!(f, "key manager: {e}"),
        }
    }
}

impl std::error::Error for WalletError {}

impl From<KeyError> for WalletError {
    fn from(e: KeyError) -> Self {
        WalletError::Key(e)
    }
}

impl WalletError {
    /// A short machine-readable name of the failed check, used as a refusal reason.
    pub fn check(&self) -> &'static str {
        match self {
            WalletError::Exists(_) => "exists",
            WalletError::UnknownPlugin { .. } => "unknown-plugin",
            WalletError::UnknownCredential(_) => "unknown-credential",
            WalletError::Integrity(_) => "integrity",
            WalletError::Malformed(_) => "malformed",
            WalletError::BadSignature(_) => "signature",
            WalletError::UntrustedIssuer(_) => "untrusted-issuer",
            WalletError::HolderBinding(_) => "holder-binding",
            WalletError::WrongAudience(_) => "audience",
            WalletError::Nonce(_) => "nonce",
            WalletError::UnrequestedClaim(_) => "unrequested-claim",
            WalletError::Revoked(_) => "revoked",
            WalletError::NotValidNow(_) => "validity",
            WalletError::StatusUnavailable(_) => "status",
            WalletError::NoMatchingCredential(_) => "no-match",
            WalletError::ConsentRefused(_) => "consent",
            WalletError::Unmapped(_) => "unmapped",
            WalletError::Protocol(_) => "protocol",
            WalletError::Storage(_) => "storage",
            WalletError::Key(_) => "key",
        }
    }
}
