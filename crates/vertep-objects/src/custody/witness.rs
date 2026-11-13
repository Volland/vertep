//! Custodians as witnesses of their object's `did:webvh` log (M8 design D4).
//!
//! A witness request rides the custody transports like every other custody request. Each
//! replica answers it with an [`EntryWitness`] policy, given the configurations of the custody
//! record the replica accepted, so a custodian witnesses only entries that declare a custodian
//! set it agreed to. The policy that understands `did:webvh` lives in `vertep-webvh`; this
//! crate only carries the request.

use super::config::CustodyConfig;
use super::wire::WitnessRequest;

/// Decides whether a custodian witnesses a log entry, and makes the proof.
pub trait EntryWitness: Send + Sync {
    /// Judges `request` given `accepted`, the effective and (while a change is in progress)
    /// pending configurations of the custody record the replica accepted. Returns the witness
    /// proof as JSON.
    ///
    /// # Errors
    ///
    /// Why the custodian does not witness the entry.
    fn witness(
        &self,
        request: &WitnessRequest,
        accepted: &[CustodyConfig],
    ) -> Result<serde_json::Value, String>;
}

/// Asks an object's custodians to witness a log entry.
pub trait WitnessSource: Send + Sync {
    /// Sends `request` to every custodian reachable and returns each one's answer: a proof or
    /// why there is none.
    fn request_witnesses(
        &self,
        request: &WitnessRequest,
    ) -> Vec<(String, Result<serde_json::Value, String>)>;
}
