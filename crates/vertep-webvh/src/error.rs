//! Errors of `did:webvh` authoring, verification and publishing.

use core::fmt;

use vertep_federation::KeyError;

/// Why a `did:webvh` operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebvhError {
    /// The key manager could not provide or use a key.
    Keys(String),
    /// The log library refused to create an entry (an unauthorised key, an unannounced
    /// pre-rotation key, a deactivated DID, invalid parameters).
    Refused(String),
    /// A log entry failed verification.
    Invalid {
        /// The `versionId` of the first failing entry, when one can be named.
        version_id: Option<String>,
        /// Why.
        reason: String,
    },
    /// Too few witness proofs to publish an entry.
    Unwitnessed {
        /// The entry's `versionId`.
        version_id: String,
        /// Proofs held.
        have: usize,
        /// Proofs required (the threshold).
        need: usize,
        /// Why witnesses asked did not give a proof (`<custodian>: <reason>`).
        refused: Vec<String>,
    },
    /// A malformed log, witness file or DID.
    Malformed(String),
    /// A file could not be read or written.
    Io(String),
    /// A publisher's external step (git, a command) failed.
    Publish(String),
}

impl fmt::Display for WebvhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WebvhError::Keys(e) => write!(f, "key manager: {e}"),
            WebvhError::Refused(e) => write!(f, "refused: {e}"),
            WebvhError::Invalid {
                version_id: Some(v),
                reason,
            } => write!(f, "log entry {v} is invalid: {reason}"),
            WebvhError::Invalid {
                version_id: None,
                reason,
            } => write!(f, "invalid log: {reason}"),
            WebvhError::Unwitnessed {
                version_id,
                have,
                need,
                refused,
            } => {
                write!(
                    f,
                    "log entry {version_id} has {have} of {need} witness proofs; {} missing",
                    need.saturating_sub(*have)
                )?;
                if !refused.is_empty() {
                    write!(f, " (refused: {})", refused.join("; "))?;
                }
                Ok(())
            }
            WebvhError::Malformed(e) => write!(f, "malformed: {e}"),
            WebvhError::Io(e) => write!(f, "{e}"),
            WebvhError::Publish(e) => write!(f, "publish failed: {e}"),
        }
    }
}

impl std::error::Error for WebvhError {}

impl From<KeyError> for WebvhError {
    fn from(e: KeyError) -> Self {
        WebvhError::Keys(e.to_string())
    }
}

pub(crate) fn io(path: &std::path::Path, e: impl fmt::Display) -> WebvhError {
    WebvhError::Io(format!("{}: {e}", path.display()))
}
