//! Hash-chained, signed agent journals ([[openspec:decision-records#Agent Journal Integrity]]):
//! an agent journal in a store whose every entry carries the same custody link, content hash and
//! signature a durable object's entries carry, so one verifier ([`crate::verify_chain`]) accepts
//! chains written by either.
//!
//! The agent opts in by building its [`vertep_agent::StoreAgentJournal`] with a
//! [`ChainedAgentSealer`]. An actor type that does not opt in uses the plain constructor and its
//! journal is unchanged. The actor's address plays the part of the object's address, the epoch
//! is fixed (1 unless set), and the writer is the signer's DID.

use std::sync::Arc;

use oxrdf::NamedNode;
use vertep_agent::EntrySealer;
use vertep_federation::PublicKey;
use vertep_store::{Entry, JournalStore, Position};

use crate::chain::{
    ChainHead, ChainLink, EntrySigner, HostKeys, VerifyFailure, read_link, seal, verify_chain,
};

/// Seals the entries of an agent journal with a chain link and the signer's signature.
pub struct ChainedAgentSealer {
    signer: Arc<dyn EntrySigner>,
    epoch: u64,
}

impl ChainedAgentSealer {
    /// A sealer that signs as `signer`, in epoch 1.
    pub fn new(signer: Arc<dyn EntrySigner>) -> Self {
        ChainedAgentSealer { signer, epoch: 1 }
    }

    /// The same sealer writing in `epoch`; it must not decrease along a journal.
    pub fn in_epoch(mut self, epoch: u64) -> Self {
        self.epoch = epoch;
        self
    }
}

impl EntrySealer for ChainedAgentSealer {
    // @tg: implements:: [[openspec:decision-records#Agent Journal Integrity]]
    fn seal(
        &self,
        actor: &NamedNode,
        position: Position,
        entry: Entry,
        previous: Option<&Entry>,
    ) -> Result<Entry, String> {
        let prev = match previous {
            Some(p) => Some(read_link(p).map_err(|e| e.to_string())?.hash),
            None => None,
        };
        let link = ChainLink {
            object: actor.clone(),
            position,
            epoch: self.epoch,
            writer: self.signer.writer().to_owned(),
            prev,
        };
        seal(&entry, &link, &*self.signer).map_err(|e| e.to_string())
    }
}

/// The one writer of an agent's journal and its keys, for [`verify_agent_journal`].
#[derive(Clone, Debug)]
pub struct AgentWriter {
    /// The writer's DID.
    pub did: String,
    /// Its verification keys.
    pub keys: Vec<PublicKey>,
}

impl HostKeys for AgentWriter {
    fn host_keys(&self, _object: &NamedNode, _epoch: u64, writer: &str) -> Option<Vec<PublicKey>> {
        (writer == self.did).then(|| self.keys.clone())
    }
}

/// Verifies the whole journal of `actor` in `store`: every content hash, signature and previous
/// hash, with no gaps. Returns the verified head.
///
/// # Errors
///
/// The first [`VerifyFailure`]; a store that cannot be read is a failure at position 1.
// @tg: implements:: [[openspec:decision-records#Agent Journal Integrity]]
pub fn verify_agent_journal(
    store: &dyn JournalStore,
    actor: &NamedNode,
    keys: &dyn HostKeys,
) -> Result<ChainHead, VerifyFailure> {
    let records = store
        .read(actor, Position::ZERO)
        .map_err(|e| VerifyFailure {
            position: Position::new(1),
            reason: crate::chain::FailureReason::Malformed(e.to_string()),
        })?;
    verify_chain(actor, ChainHead::EMPTY, &records, keys)
}
