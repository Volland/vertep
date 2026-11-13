//! A `did:webvh` DID log with its witness proofs, and its verification.
//!
//! Verification has three steps:
//!
//! 1. Each entry is replayed against the one before it (hash chain, SCID, update-key
//!    authorisation, proofs, `versionTime`), which names the first failing entry.
//! 2. Every witnessed entry is checked for a threshold of witness proofs, which says how many
//!    are missing.
//! 3. The whole log and its witness file go through the `didwebvh-rs` resolver, the path
//!    `vertep-federation` resolves `did:webvh` with.

use std::collections::BTreeMap;

use didwebvh_rs::DIDWebVHState;
use didwebvh_rs::log_entry_state::LogEntryState;
use didwebvh_rs::prelude::WitnessProofCollection;
use didwebvh_rs::witness::Witnesses;
use serde_json::Value;

use crate::error::WebvhError;

/// A DID log: its entries (one JSON object each, `did.jsonl` order) and its witness proofs
/// (`did-witness.json`).
#[derive(Clone, Debug)]
pub struct DidLog {
    did: String,
    entries: Vec<Value>,
    witness: Vec<Value>,
}

/// What a verified log says.
#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    /// The DID.
    pub did: String,
    /// The current DID document (the last entry's `state`).
    pub document: Value,
    /// The last entry's `versionId`.
    pub version_id: String,
    /// The number of entries.
    pub versions: u32,
    /// Whether the DID is deactivated.
    pub deactivated: bool,
    /// The `ttl` parameter in force, in seconds (`did:webvh` defaults to 3600).
    pub ttl: u32,
    /// The witnesses in force after the last entry (their `did:key` DIDs) and the threshold.
    pub witnesses: Option<(Vec<String>, u32)>,
}

pub(crate) fn witnesses_of(w: Option<&Witnesses>) -> Option<(Vec<String>, u32)> {
    let w = w?;
    let list = w.witnesses()?;
    if list.is_empty() {
        return None;
    }
    Some((
        list.iter().map(|x| x.as_did()).collect(),
        w.threshold().unwrap_or(0),
    ))
}

fn malformed(e: impl std::fmt::Display) -> WebvhError {
    WebvhError::Malformed(e.to_string())
}

impl DidLog {
    /// A log of `entries` (with no witness proofs yet) for `did`.
    pub fn new(did: impl Into<String>, entries: Vec<Value>) -> Self {
        DidLog {
            did: did.into(),
            entries,
            witness: Vec::new(),
        }
    }

    /// Reads a log from `did.jsonl` text and the optional `did-witness.json` text. The DID is
    /// the last entry's document `id`.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Malformed`] for lines that are not JSON objects, an empty log, or a
    /// witness file that is not an array.
    // @tg: implements:: [[openspec:webvh-hosting#Third-Party Logs Interoperate]] {note: "any conforming did:webvh 1.0 log, verified and published like Vertep's own"}
    pub fn from_files(log: &str, witness: Option<&str>) -> Result<Self, WebvhError> {
        let mut entries = Vec::new();
        for (i, line) in log.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line)
                .map_err(|e| malformed(format!("did.jsonl line {}: {e}", i + 1)))?;
            if !v.is_object() {
                return Err(malformed(format!(
                    "did.jsonl line {} is not an object",
                    i + 1
                )));
            }
            entries.push(v);
        }
        let did = entries
            .last()
            .and_then(|e| e.pointer("/state/id"))
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("did.jsonl has no entry with a state id"))?
            .to_owned();
        let witness = match witness {
            Some(w) if !w.trim().is_empty() => {
                let v: Value = serde_json::from_str(w)
                    .map_err(|e| malformed(format!("did-witness.json: {e}")))?;
                v.as_array()
                    .cloned()
                    .ok_or_else(|| malformed("did-witness.json is not an array"))?
            }
            _ => Vec::new(),
        };
        Ok(DidLog {
            did,
            entries,
            witness,
        })
    }

    /// The DID.
    pub fn did(&self) -> &str {
        &self.did
    }

    /// The entries.
    pub fn entries(&self) -> &[Value] {
        &self.entries
    }

    /// The number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the log has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The last entry's `versionId`.
    pub fn version_id(&self) -> Option<&str> {
        self.entries
            .last()
            .and_then(|e| e.get("versionId"))
            .and_then(Value::as_str)
    }

    /// The last entry's document.
    pub fn document(&self) -> Option<&Value> {
        self.entries.last().and_then(|e| e.get("state"))
    }

    /// The log as JSON Lines, one entry per line, as `did.jsonl` holds it.
    pub fn jsonl(&self) -> String {
        let mut s = String::new();
        for e in &self.entries {
            s.push_str(&e.to_string());
            s.push('\n');
        }
        s
    }

    /// The log without its last entry (with the same witness proofs).
    pub fn without_last(&self) -> DidLog {
        DidLog {
            did: self.did.clone(),
            entries: self.entries[..self.entries.len().saturating_sub(1)].to_vec(),
            witness: self.witness.clone(),
        }
    }

    /// The witness proofs as `did-witness.json` holds them, or `None` when there are none.
    pub fn witness_json(&self) -> Option<String> {
        (!self.witness.is_empty()).then(|| Value::Array(self.witness.clone()).to_string())
    }

    /// The witness proof records (`{"versionId", "proof": [...]}`).
    pub fn witness_proofs(&self) -> &[Value] {
        &self.witness
    }

    pub(crate) fn push_entry(&mut self, entry: Value) {
        self.entries.push(entry);
    }

    /// Adds a witness proof for the entry `version_id`. A witness keeps only its newest proof,
    /// which also attests every earlier entry (`did:webvh` 1.0).
    ///
    /// # Errors
    ///
    /// [`WebvhError::Malformed`] when the proof is not a Data Integrity proof or the version
    /// id is not `<n>-<hash>`.
    pub fn add_witness_proof(&mut self, version_id: &str, proof: &Value) -> Result<(), WebvhError> {
        let mut coll: WitnessProofCollection =
            serde_json::from_value(Value::Array(self.witness.clone())).map_err(malformed)?;
        coll.generate_proof_state(u32::MAX).map_err(malformed)?;
        let p: affinidi_data_integrity::DataIntegrityProof =
            serde_json::from_value(proof.clone()).map_err(malformed)?;
        coll.add_proof(version_id, &p, false).map_err(malformed)?;
        coll.write_optimise_records().map_err(malformed)?;
        let v = serde_json::to_value(&coll).map_err(malformed)?;
        self.witness = v.as_array().cloned().unwrap_or_default();
        Ok(())
    }

    /// Replays every entry against the one before it, without witness checks.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Invalid`] naming the first entry that fails.
    pub fn replay(&self) -> Result<DIDWebVHState, WebvhError> {
        let mut parsed =
            DIDWebVHState::parse_log_entries(&self.jsonl()).map_err(|e| WebvhError::Invalid {
                version_id: None,
                reason: e.to_string(),
            })?;
        let mut done: Vec<LogEntryState> = Vec::with_capacity(parsed.len());
        for mut e in parsed.drain(..) {
            if let Err(err) = e.verify_log_entry(done.last()) {
                return Err(WebvhError::Invalid {
                    version_id: Some(e.get_version_id().to_owned()),
                    reason: err.to_string(),
                });
            }
            done.push(e);
        }
        if let Some(first) = done.first() {
            let id = first.get_state().get("id").and_then(Value::as_str);
            if id.is_some_and(|id| id != self.did) && done.len() == 1 {
                return Err(WebvhError::Invalid {
                    version_id: Some(first.get_version_id().to_owned()),
                    reason: format!("the entry's DID is not {}", self.did),
                });
            }
        }
        let mut state = DIDWebVHState::default();
        *state.log_entries_mut() = done;
        Ok(state)
    }

    /// The newest version number each witness (by verification method) has a proof for.
    fn witness_heads(&self) -> BTreeMap<String, u32> {
        let mut heads = BTreeMap::new();
        for rec in &self.witness {
            let n = rec
                .get("versionId")
                .and_then(Value::as_str)
                .and_then(|v| v.split_once('-'))
                .and_then(|(n, _)| n.parse::<u32>().ok())
                .unwrap_or(0);
            for p in rec
                .get("proof")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(vm) = p.get("verificationMethod").and_then(Value::as_str) {
                    let h = heads.entry(vm.to_owned()).or_insert(0);
                    *h = (*h).max(n);
                }
            }
        }
        heads
    }

    /// For every entry that needs witnessing: its `versionId`, the proofs it has and the
    /// threshold, from the replayed `state`.
    pub(crate) fn witness_counts(
        state: &DIDWebVHState,
        log: &DidLog,
    ) -> Vec<(String, usize, usize)> {
        let heads = log.witness_heads();
        let mut out = Vec::new();
        for e in state.log_entries() {
            let Some(w) = e.get_active_witnesses() else {
                continue;
            };
            let Some(list) = w.witnesses() else { continue };
            let n = e
                .get_version_id()
                .split_once('-')
                .and_then(|(n, _)| n.parse::<u32>().ok())
                .unwrap_or(0);
            let have = list
                .iter()
                .filter(|x| heads.get(&x.as_did_key()).is_some_and(|h| *h >= n))
                .count();
            let need = w.threshold().unwrap_or(0) as usize;
            out.push((e.get_version_id().to_owned(), have, need));
        }
        out
    }

    /// Proofs held and needed for the last entry, when it needs witnessing.
    ///
    /// # Errors
    ///
    /// As [`DidLog::replay`].
    pub fn last_entry_witnessing(&self) -> Result<Option<(usize, usize)>, WebvhError> {
        let state = self.replay()?;
        let last = self.version_id().unwrap_or_default().to_owned();
        Ok(Self::witness_counts(&state, self)
            .into_iter()
            .find(|(v, ..)| *v == last)
            .map(|(_, have, need)| (have, need)))
    }

    /// Verifies the whole log with its witness proofs.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Invalid`] naming the first failing entry; [`WebvhError::Unwitnessed`] for
    /// the first entry without a threshold of witness proofs.
    // @tg: implements:: [[openspec:webvh-identity#Verify Before Publish]] {note: "the check every publisher runs"}
    pub fn verify(&self) -> Result<Verified, WebvhError> {
        let state = self.replay()?;
        for (version_id, have, need) in Self::witness_counts(&state, self) {
            if have < need {
                return Err(WebvhError::Unwitnessed {
                    version_id,
                    have,
                    need,
                    refused: Vec::new(),
                });
            }
        }
        let mut resolved = DIDWebVHState::default();
        let witness = self.witness_json();
        let (entry, meta) = futures::executor::block_on(resolved.resolve_log_owned(
            &self.did,
            &self.jsonl(),
            witness.as_deref(),
        ))
        .map_err(|e| WebvhError::Invalid {
            version_id: None,
            reason: e.to_string(),
        })?;
        use didwebvh_rs::log_entry::LogEntryMethods as _;
        let last = resolved.log_entries().last();
        let ttl = last
            .and_then(|e| e.validated_parameters.ttl)
            .unwrap_or(3600);
        let witnesses = last.and_then(|e| witnesses_of(e.validated_parameters.witness.as_deref()));
        Ok(Verified {
            did: self.did.clone(),
            document: entry.get_state().clone(),
            version_id: meta.version_id.clone(),
            versions: meta.version_number,
            deactivated: meta.deactivated,
            ttl,
            witnesses,
        })
    }

    /// The `did:web` document for this DID (the current document under its `did:web` id), as
    /// `did.json` beside the log.
    ///
    /// # Errors
    ///
    /// As [`DidLog::verify`].
    pub fn did_web_document(&self) -> Result<Value, WebvhError> {
        let mut resolved = DIDWebVHState::default();
        let witness = self.witness_json();
        futures::executor::block_on(resolved.resolve_log_owned(
            &self.did,
            &self.jsonl(),
            witness.as_deref(),
        ))
        .map_err(|e| WebvhError::Invalid {
            version_id: None,
            reason: e.to_string(),
        })?;
        resolved.to_web_did().map_err(|e| WebvhError::Invalid {
            version_id: None,
            reason: e.to_string(),
        })
    }
}
