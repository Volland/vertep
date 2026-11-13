//! Custodians as witnesses of durable-object DID logs (feature `custody`).
//!
//! An object's DID log names every custodian as a witness, by the `did:key` of its Ed25519
//! signing key, with the custody quorum as the threshold ([`custody_witnessing`]). A custodian
//! witnesses an entry through [`CustodianWitness`] only when the entry is the valid next entry,
//! signed by a current update key, and declares a configuration the custodian accepted.
//!
//! A custody change carries its log entry ([`custody_change`]); after the joint-consensus
//! switch the writer's [`WebvhPublisher`] collects proofs from the custodians and publishes.

use std::sync::Arc;

use affinidi_data_integrity::{DataIntegrityProof, SignOptions};
use serde_json::{Value, json};
use vertep_federation::{DidDocument, KeyManager};
use vertep_objects::EntrySigner;
use vertep_objects::custody::{
    CustodyConfig, DocumentPublisher, EntryWitness, KeyDirectory, ReconfigurationRequest,
    WitnessRequest, WitnessSource,
};

use crate::author::{Author, EntryChange, Witnessing};
use crate::document::with_custody;
use crate::error::WebvhError;
use crate::log::DidLog;
use crate::publish::{LogPublisher, Published};
use crate::signer::{KmSigner, did_key};

/// The witness list of an object with custody configuration `config`: each custodian's
/// `did:key` (from the first verification key `directory` knows for it), in configuration
/// order, with the quorum as the threshold.
///
/// # Errors
///
/// [`WebvhError::Keys`] when the directory knows no key for a custodian.
// @tg: implements:: [[openspec:webvh-identity#Custodians Witness Object Logs]]
pub fn custody_witnessing(
    config: &CustodyConfig,
    directory: &dyn KeyDirectory,
) -> Result<Witnessing, WebvhError> {
    let mut witnesses = Vec::with_capacity(config.custodians.len());
    for c in &config.custodians {
        let key = directory
            .verification_keys(&c.id)
            .and_then(|k| k.first().copied())
            .ok_or_else(|| {
                WebvhError::Keys(format!("no signing key known for custodian {}", c.id))
            })?;
        witnesses.push(did_key(&key));
    }
    Ok(Witnessing {
        witnesses,
        threshold: u32::try_from(config.quorum).unwrap_or(u32::MAX),
    })
}

/// A custodian's witnessing policy: the checks of *Custodian Witnessing Rules*, and the proof
/// made with the custodian's signing key through its key manager.
pub struct CustodianWitness {
    signer: KmSigner,
}

impl std::fmt::Debug for CustodianWitness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustodianWitness")
            .field("witness", &self.signer.did_key())
            .finish()
    }
}

impl CustodianWitness {
    /// The policy of the custodian whose signing key is `key` in `keys`.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Keys`] when the key is unknown or not Ed25519.
    pub fn new(keys: Arc<dyn KeyManager>, key: impl Into<String>) -> Result<Self, WebvhError> {
        Ok(CustodianWitness {
            signer: KmSigner::new(keys, key)?,
        })
    }

    /// The `did:key` this custodian witnesses as.
    pub fn did_key(&self) -> String {
        self.signer.did_key()
    }

    /// A witness proof for the entry `version_id`, without any check (the `vt webvh witness`
    /// command, where the operator decides).
    ///
    /// # Errors
    ///
    /// [`WebvhError::Keys`] when the key manager cannot sign.
    pub fn prove(&self, version_id: &str) -> Result<Value, WebvhError> {
        prove(&self.signer, version_id)
    }
}

pub(crate) fn prove(signer: &KmSigner, version_id: &str) -> Result<Value, WebvhError> {
    let proof = futures::executor::block_on(DataIntegrityProof::sign(
        &json!({ "versionId": version_id }),
        signer,
        SignOptions::new(),
    ))
    .map_err(|e| WebvhError::Keys(e.to_string()))?;
    serde_json::to_value(proof).map_err(|e| WebvhError::Malformed(e.to_string()))
}

fn describe(c: &CustodyConfig) -> String {
    format!(
        "version {} with {} custodians and quorum {}",
        c.version,
        c.custodians.len(),
        c.quorum
    )
}

impl EntryWitness for CustodianWitness {
    // @tg: implements:: [[openspec:webvh-identity#Custodian Witnessing Rules]]
    fn witness(
        &self,
        request: &WitnessRequest,
        accepted: &[CustodyConfig],
    ) -> Result<Value, String> {
        let mut log = if request.log.trim().is_empty() {
            DidLog::new(request.object.clone(), Vec::new())
        } else {
            let prior = DidLog::from_files(&request.log, request.witness.as_deref())
                .map_err(|e| e.to_string())?;
            if prior.did() != request.object {
                return Err(format!(
                    "the log is {}, not {}",
                    prior.did(),
                    request.object
                ));
            }
            prior
                .verify()
                .map_err(|e| format!("the published log does not verify: {e}"))?;
            prior
        };
        log.push_entry(request.entry.clone());
        let state = log
            .replay()
            .map_err(|e| format!("not a valid next entry signed by a current update key: {e}"))?;
        let last = state
            .log_entries()
            .last()
            .ok_or_else(|| "no entry to witness".to_owned())?;
        if last.get_state().get("id").and_then(Value::as_str) != Some(request.object.as_str()) {
            return Err(format!("the entry is not a document of {}", request.object));
        }
        let doc = DidDocument::from_json(last.get_state())
            .map_err(|e| format!("the entry's document: {e}"))?;
        let declared = CustodyConfig::from_document(&doc).map_err(|e| e.to_string())?;
        if !accepted.contains(&declared) {
            return Err(format!(
                "the entry declares custodian set {}, which this custodian has not accepted (it holds {})",
                describe(&declared),
                accepted
                    .iter()
                    .map(describe)
                    .collect::<Vec<_>>()
                    .join(" and ")
            ));
        }
        let me = self.signer.did_key();
        let listed = last
            .get_active_witnesses()
            .and_then(|w| w.witnesses().map(|l| l.iter().any(|x| x.as_did() == me)))
            .unwrap_or(false);
        if !listed {
            return Err(format!("{me} is not a witness of this entry"));
        }
        prove(&self.signer, last.get_version_id()).map_err(|e| e.to_string())
    }
}

/// Collects witness proofs for the last entry of `log` from `source` until its threshold is
/// met, verifies the log and publishes it through `publisher`.
///
/// # Errors
///
/// [`WebvhError::Unwitnessed`] with the number of missing proofs and the custodians'
/// refusals, in which case nothing is published; the publisher's errors.
pub fn publish_witnessed(
    publisher: &dyn LogPublisher,
    mut log: DidLog,
    source: &dyn WitnessSource,
) -> Result<Published, WebvhError> {
    let version_id = log
        .version_id()
        .ok_or_else(|| WebvhError::Malformed("an empty log".into()))?
        .to_owned();
    let mut refused = Vec::new();
    if let Some((have, need)) = log.last_entry_witnessing()?
        && have < need
    {
        let prior = log.without_last();
        let request = WitnessRequest {
            object: log.did().to_owned(),
            log: prior.jsonl(),
            witness: prior.witness_json(),
            entry: log.entries().last().cloned().unwrap_or_default(),
        };
        for (custodian, answer) in source.request_witnesses(&request) {
            match answer
                .map_err(WebvhError::Refused)
                .and_then(|proof| log.add_witness_proof(&version_id, &proof))
            {
                Ok(()) => {}
                Err(e) => refused.push(format!("{custodian}: {e}")),
            }
        }
        if let Some((have, need)) = log.last_entry_witnessing()?
            && have < need
        {
            return Err(WebvhError::Unwitnessed {
                version_id,
                have,
                need,
                refused,
            });
        }
    }
    publisher.publish(&log)
}

/// Publishes the log entries of custody changes: the custody [`DocumentPublisher`] of an
/// object whose DID log `publisher` hosts.
pub struct WebvhPublisher {
    publisher: Arc<dyn LogPublisher>,
}

impl std::fmt::Debug for WebvhPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebvhPublisher").finish_non_exhaustive()
    }
}

impl WebvhPublisher {
    /// Publishes through `publisher`.
    pub fn new(publisher: Arc<dyn LogPublisher>) -> Self {
        WebvhPublisher { publisher }
    }
}

impl DocumentPublisher for WebvhPublisher {
    // @tg: implements:: [[openspec:webvh-identity#Custody Changes Publish As Log Entries]]
    fn publish(
        &self,
        object: &str,
        log_entry: &Value,
        witnesses: &dyn WitnessSource,
    ) -> Result<(), String> {
        let current = self
            .publisher
            .current(object)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no published log for {object}"))?;
        let version_id = log_entry.get("versionId").and_then(Value::as_str);
        let mut log = current.clone();
        if current.version_id() != version_id {
            log.push_entry(log_entry.clone());
        }
        publish_witnessed(self.publisher.as_ref(), log, witnesses)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Prepares a change of an object's custodian set to `new`: the next log entry (signed by
/// `author`, declaring `new` and its witness list) appended to `log`, and the reconfiguration
/// request carrying it, signed by `controller`. Nothing is published.
///
/// # Errors
///
/// [`WebvhError::Keys`] when `directory` lacks a custodian's key; the errors of
/// [`Author::append`]; [`WebvhError::Keys`] when the controller cannot sign.
// @tg: implements:: [[openspec:webvh-identity#Custody Changes Publish As Log Entries]] {note: "the controller's side"}
pub fn custody_change(
    author: &mut Author,
    log: &mut DidLog,
    new: &CustodyConfig,
    directory: &dyn KeyDirectory,
    controller: &dyn EntrySigner,
) -> Result<ReconfigurationRequest, WebvhError> {
    let current = log
        .document()
        .cloned()
        .ok_or_else(|| WebvhError::Malformed("an empty log".into()))?;
    let document = with_custody(&current, new)?;
    let witnessing = custody_witnessing(new, directory)?;
    author.append(
        log,
        EntryChange {
            document: Some(document),
            witnesses: Some(Some(witnessing)),
            ..EntryChange::default()
        },
    )?;
    let entry = log.entries().last().cloned();
    ReconfigurationRequest {
        object: log.did().to_owned(),
        config: new.clone(),
        did_log_entry: entry,
        signature: String::new(),
    }
    .sign(controller)
    .map_err(WebvhError::from)
}
