//! `did:webvh` (feature `webvh`): the DID log is fetched through the [`HttpFetcher`] and its chain
//! of entries is verified by the DIF `didwebvh-rs` implementation (entry hashes, SCID, update-key
//! authorisation and Data Integrity proofs, and witness proofs when the log requires them). The
//! current document is returned only if the whole log verifies.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use didwebvh_rs::DIDWebVHState;
use didwebvh_rs::log_entry::LogEntryMethods as _;

use crate::did::{Did, DidDocument};

use super::web::{fetch_error, web_url};
use super::{DidResolver, HttpFetcher, Resolution, ResolutionError};

/// The `did:webvh` resolver.
#[derive(Clone)]
pub struct WebVhResolver {
    fetcher: Arc<dyn HttpFetcher>,
    timeout: Duration,
}

impl fmt::Debug for WebVhResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebVhResolver")
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// The URL of a `did:webvh` resource (`did.jsonl`, `did-witness.json`).
pub fn webvh_url(did: &Did, file: &str) -> Option<String> {
    // did:webvh:<SCID>:<host>[:<path>...]
    let (_scid, rest) = did.method_specific_id().split_once(':')?;
    web_url(rest, file)
}

/// Verifies the DID log `log` (JSON Lines) of `did`, with the optional witness proofs, and
/// returns the current document and version id.
///
/// # Errors
///
/// [`ResolutionError::VerificationFailed`] when any entry of the log fails verification.
pub fn verify_log(
    did: &Did,
    log: &str,
    witness: Option<&str>,
) -> Result<Resolution, ResolutionError> {
    let mut state = DIDWebVHState::default();
    let (entry, meta) =
        futures::executor::block_on(state.resolve_log_owned(did.as_str(), log, witness))
            .map_err(|e| ResolutionError::VerificationFailed(e.to_string()))?;
    let doc_json = entry.get_state().clone();
    let document = DidDocument::from_json(&doc_json)
        .map_err(|e| ResolutionError::InvalidDocument(e.to_string()))?;
    Ok(Resolution {
        document,
        version_id: Some(meta.version_id.to_string()),
    })
}

impl WebVhResolver {
    /// A resolver fetching through `fetcher`, giving up after `timeout` per file.
    pub fn new(fetcher: Arc<dyn HttpFetcher>, timeout: Duration) -> Self {
        WebVhResolver { fetcher, timeout }
    }
}

impl DidResolver for WebVhResolver {
    // @tg: implements:: [[openspec:did-identity#Supported DID Methods]] {confidence: partial, note: "did:webvh with log verification"}
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        if did.method() != "webvh" {
            return Err(ResolutionError::MethodNotSupported(did.method().to_owned()));
        }
        let url = webvh_url(did, "did.jsonl")
            .ok_or_else(|| ResolutionError::InvalidDid(did.to_string()))?;
        let body = self
            .fetcher
            .get(&url, self.timeout)
            .map_err(|e| fetch_error(&url, e))?;
        let log = String::from_utf8(body)
            .map_err(|e| ResolutionError::InvalidDocument(format!("{url}: {e}")))?;
        let entries = DIDWebVHState::parse_log_entries(&log)
            .map_err(|e| ResolutionError::VerificationFailed(e.to_string()))?;
        let witness = if DIDWebVHState::needs_witness_proofs(&entries) {
            let wurl = webvh_url(did, "did-witness.json")
                .ok_or_else(|| ResolutionError::InvalidDid(did.to_string()))?;
            let w = self
                .fetcher
                .get(&wurl, self.timeout)
                .map_err(|e| fetch_error(&wurl, e))?;
            Some(
                String::from_utf8(w)
                    .map_err(|e| ResolutionError::InvalidDocument(format!("{wurl}: {e}")))?,
            )
        } else {
            None
        };
        verify_log(did, &log, witness.as_deref())
    }
}
