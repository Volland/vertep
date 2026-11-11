//! Errors of the storage contracts.

use core::fmt;

use oxrdf::NamedNode;

use crate::journal::Position;

/// An error of a read, a graph store operation or a backend outside the append protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The store cannot be reached right now (a remote endpoint is down, a file is busy). The
    /// operation may succeed when retried.
    Unavailable(String),
    /// The request is malformed: an invalid entry, a quad the store cannot hold, invalid SPARQL.
    Invalid(String),
    /// The store does not implement this operation or this kind of term.
    Unsupported(String),
    /// Any other failure inside the backend.
    Backend(String),
}

impl StoreError {
    /// An [`StoreError::Invalid`] from any message.
    pub fn invalid(msg: impl Into<String>) -> Self {
        StoreError::Invalid(msg.into())
    }

    /// A [`StoreError::Backend`] from any error.
    pub fn backend(err: impl fmt::Display) -> Self {
        StoreError::Backend(err.to_string())
    }

    /// True for [`StoreError::Unavailable`]: retrying later may succeed.
    pub fn is_transient(&self) -> bool {
        matches!(self, StoreError::Unavailable(_))
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Unavailable(m) => write!(f, "store unavailable: {m}"),
            StoreError::Invalid(m) => write!(f, "invalid request: {m}"),
            StoreError::Unsupported(m) => write!(f, "unsupported: {m}"),
            StoreError::Backend(m) => write!(f, "backend error: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Why a journal append wrote nothing.
///
/// Every variant means the same thing for the journal: none of the append's entries, and none
/// of its state changes, are visible to any later read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppendError {
    /// The actor's journal is not at the expected position (compare-and-set failed).
    Conflict {
        /// The position the append expected.
        expected: Position,
        /// The position the journal is actually at.
        actual: Position,
    },
    /// The fencing token is older than the highest epoch the store has accepted for its scope.
    Fenced {
        /// The scope of the token.
        scope: NamedNode,
        /// The epoch the append carried.
        epoch: u64,
        /// The highest epoch accepted so far for the scope.
        highest: u64,
    },
    /// The append carried a fencing token or a state delta, and the store does not support it
    /// (see [`crate::JournalCapabilities`]).
    Unsupported(String),
    /// The entries are malformed (empty append, wrong graph, duplicate entry IRI, a term the
    /// store cannot hold).
    Invalid(String),
    /// The store failed; the transaction was rolled back.
    Storage(StoreError),
}

impl AppendError {
    /// True for [`AppendError::Conflict`].
    pub fn is_conflict(&self) -> bool {
        matches!(self, AppendError::Conflict { .. })
    }

    /// True for [`AppendError::Fenced`].
    pub fn is_fenced(&self) -> bool {
        matches!(self, AppendError::Fenced { .. })
    }
}

impl fmt::Display for AppendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppendError::Conflict { expected, actual } => write!(
                f,
                "append conflict: expected position {}, journal is at {}",
                expected.get(),
                actual.get()
            ),
            AppendError::Fenced {
                scope,
                epoch,
                highest,
            } => write!(
                f,
                "append fenced: epoch {epoch} for {scope} is below the accepted epoch {highest}"
            ),
            AppendError::Unsupported(m) => write!(f, "unsupported append: {m}"),
            AppendError::Invalid(m) => write!(f, "invalid append: {m}"),
            AppendError::Storage(e) => write!(f, "append failed: {e}"),
        }
    }
}

impl std::error::Error for AppendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AppendError::Storage(e) => Some(e),
            _ => None,
        }
    }
}

impl From<StoreError> for AppendError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Invalid(m) => AppendError::Invalid(m),
            StoreError::Unsupported(m) => AppendError::Unsupported(m),
            other => AppendError::Storage(other),
        }
    }
}
