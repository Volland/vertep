//! The explanation bundle: one actor's journal up to a position, the chart versions it uses
//! addressed by hash, and a manifest of digests, verifiable offline
//! ([[openspec:decision-records#Explanation Bundle]]).
//!
//! Verification needs no decider, no network and no runtime: it re-hashes every chart, checks
//! every entry against the manifest, replays the journal into a fresh host and recomputes the
//! explanation of each position in the bundle's range, comparing each with its digest.
//!
//! What a bundle proves and what it does not: it is self-consistent, so an altered entry, a
//! missing or altered chart and a changed explanation are all detected and named. It is not
//! signed, so a party who rewrites the entries *and* the manifest together produces a bundle
//! that verifies; binding a bundle to a signer is the job of a chained, signed journal.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::explain::Explanation;
use crate::host::{AgentHost, HostError};
use crate::journal::{AgentInput, JournalEntry, MemoryJournal};

/// The bundle format name.
pub const BUNDLE_FORMAT: &str = "vertep-explanation-bundle/1";

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A digest of one position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionDigest {
    /// The journal position.
    pub position: u64,
    /// The SHA-256 of the JSON text, hex.
    pub sha256: String,
}

/// What the bundle claims about its contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The digest of each entry's JSON, positions 1 to `to`.
    pub entries: Vec<PositionDigest>,
    /// The digest of each chart's N-Quads, by version.
    pub charts: BTreeMap<String, String>,
    /// The digest of the explanation of each position from `from` to `to`.
    pub explanations: Vec<PositionDigest>,
}

/// An exported, offline-verifiable record of one actor's journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    /// [`BUNDLE_FORMAT`].
    pub format: String,
    /// The actor.
    pub actor: String,
    /// The first position explained.
    pub from: u64,
    /// The last position; the journal slice is positions 1 to this.
    pub to: u64,
    /// The digests.
    pub manifest: Manifest,
    /// The journal slice, positions 1 to `to`.
    pub entries: Vec<JournalEntry>,
    /// The canonical N-Quads of each chart version the journal uses, by version IRI.
    pub charts: BTreeMap<String, String>,
}

/// Why a bundle did not verify, naming what is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleError {
    /// Not a bundle of this format, or not JSON.
    Format(String),
    /// The chart a journal entry cites is not in the bundle.
    MissingChart(String),
    /// A chart's text does not match the digest in the manifest.
    ChartDigest(String),
    /// A chart's text does not hash to the version that names it.
    ChartHash {
        /// The version the bundle names it by.
        claimed: String,
        /// The version its text hashes to.
        computed: String,
    },
    /// An entry does not match its digest in the manifest.
    AlteredEntry(u64),
    /// The entries are not the positions 1 to `to` in order, or their number differs.
    Entries(String),
    /// The recomputed explanation of a position does not match the manifest.
    ExplanationMismatch(u64),
    /// The journal could not be replayed.
    Replay(String),
}

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleError::Format(m) => write!(f, "not a bundle: {m}"),
            BundleError::MissingChart(v) => write!(f, "the chart {v} is missing from the bundle"),
            BundleError::ChartDigest(v) => write!(f, "the chart {v} does not match its digest"),
            BundleError::ChartHash { claimed, computed } => {
                write!(f, "the chart named {claimed} hashes to {computed}")
            }
            BundleError::AlteredEntry(p) => write!(f, "entry {p} was altered"),
            BundleError::Entries(m) => write!(f, "bad entries: {m}"),
            BundleError::ExplanationMismatch(p) => {
                write!(f, "the explanation of entry {p} differs from the manifest")
            }
            BundleError::Replay(m) => write!(f, "replay failed: {m}"),
        }
    }
}

impl std::error::Error for BundleError {}

/// What a successful verification produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    /// The recomputed explanations, positions `from` to `to`.
    pub explanations: Vec<Explanation>,
    /// How many deciders were called: always zero.
    pub decider_calls: usize,
}

impl Bundle {
    /// The bundle as JSON text.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// A bundle from JSON text.
    ///
    /// # Errors
    ///
    /// [`BundleError::Format`] for text that is not a bundle of this format.
    pub fn from_json(text: &str) -> Result<Bundle, BundleError> {
        let b: Bundle =
            serde_json::from_str(text).map_err(|e| BundleError::Format(e.to_string()))?;
        if b.format != BUNDLE_FORMAT {
            return Err(BundleError::Format(format!("format {}", b.format)));
        }
        Ok(b)
    }

    /// Verifies the bundle offline and returns the recomputed explanations.
    ///
    /// # Errors
    ///
    /// The first thing wrong, as a [`BundleError`] that names the entry or chart.
    // @tg: implements:: [[openspec:decision-records#Explanation Bundle]]
    pub fn verify(&self) -> Result<Verified, BundleError> {
        if self.format != BUNDLE_FORMAT {
            return Err(BundleError::Format(format!("format {}", self.format)));
        }
        // the chart the journal starts under must be present
        let Some(first) = self.entries.first() else {
            return Err(BundleError::Entries("the bundle has no entries".to_owned()));
        };
        let AgentInput::Start { version } = &first.input else {
            return Err(BundleError::Entries("entry 1 is not a start".to_owned()));
        };
        if !self.charts.contains_key(version) {
            return Err(BundleError::MissingChart(version.clone()));
        }
        // every chart: digest against the manifest, and hash against the name
        let mut host = AgentHost::builder(Arc::new(vertep_runtime::VirtualClock::new(0)))
            .journal(Arc::new(MemoryJournal::new()))
            .build();
        for (name, nquads) in &self.charts {
            if self.manifest.charts.get(name) != Some(&sha(nquads)) {
                return Err(BundleError::ChartDigest(name.clone()));
            }
            let chart = vertep_rdf::read_chart(nquads.as_bytes(), vertep_rdf::RdfFormat::NQuads)
                .map_err(|e| BundleError::Replay(format!("chart {name}: {e:?}")))?
                .chart;
            let computed = vertep_rdf::chart_version(&chart);
            if computed.as_str() != name {
                return Err(BundleError::ChartHash {
                    claimed: name.clone(),
                    computed: computed.as_str().to_owned(),
                });
            }
            host.register(chart)
                .map_err(|e| BundleError::Replay(format!("chart {name}: {e}")))?;
        }
        // entries: positions 1..=to, each against its digest
        if self.entries.len() as u64 != self.to || self.manifest.entries.len() as u64 != self.to {
            return Err(BundleError::Entries(format!(
                "{} entries and {} digests for positions 1 to {}",
                self.entries.len(),
                self.manifest.entries.len(),
                self.to
            )));
        }
        for (i, (entry, digest)) in self.entries.iter().zip(&self.manifest.entries).enumerate() {
            let want = i as u64 + 1;
            if entry.position != want || digest.position != want {
                return Err(BundleError::Entries(format!(
                    "position {want} is out of order"
                )));
            }
            if sha(&entry.to_json()) != digest.sha256 {
                return Err(BundleError::AlteredEntry(want));
            }
        }
        // replay into the fresh host and recompute each explanation
        let journal = host.journal_store().clone();
        for entry in &self.entries {
            journal
                .append(&self.actor, entry.clone())
                .map_err(|e| BundleError::Replay(e.to_string()))?;
        }
        let mut explanations = Vec::new();
        for want in &self.manifest.explanations {
            let x = host
                .explain(&self.actor, want.position)
                .map_err(|e: HostError| BundleError::Replay(e.to_string()))?;
            if sha(&x.to_json()) != want.sha256 {
                return Err(BundleError::ExplanationMismatch(want.position));
            }
            explanations.push(x);
        }
        if explanations.len() as u64 != self.to + 1 - self.from.max(1) {
            return Err(BundleError::Entries(format!(
                "the manifest explains {} positions, not {} to {}",
                explanations.len(),
                self.from,
                self.to
            )));
        }
        Ok(Verified {
            explanations,
            decider_calls: 0,
        })
    }
}

impl AgentHost {
    /// Exports a bundle of `actor`'s journal up to `to`, explaining positions `from` to `to`.
    ///
    /// # Errors
    ///
    /// [`HostError`] when the journal cannot be read, has no such position, or its chart version
    /// is not registered.
    // @tg: implements:: [[openspec:decision-records#Explanation Bundle]]
    pub fn export_bundle(&self, actor: &str, from: u64, to: u64) -> Result<Bundle, HostError> {
        let entries: Vec<JournalEntry> = self
            .journal_store()
            .try_read(actor)
            .map_err(HostError::Journal)?
            .into_iter()
            .filter(|e| e.position <= to)
            .collect();
        if entries.len() as u64 != to || from == 0 || from > to {
            return Err(HostError::Unknown(format!(
                "{actor} has no positions {from} to {to}"
            )));
        }
        let AgentInput::Start { version } = &entries[0].input else {
            return Err(HostError::Unknown(actor.to_owned()));
        };
        let version = version.clone();
        let chart = self
            .chart_ir(&version)
            .ok_or_else(|| HostError::MissingVersion(version.clone()))?;
        let nquads =
            vertep_rdf::write_chart(&vertep_rdf::project(&chart), vertep_rdf::RdfFormat::NQuads);
        let mut explanations = Vec::new();
        for p in from..=to {
            let x = self.explain(actor, p)?;
            explanations.push(PositionDigest {
                position: p,
                sha256: sha(&x.to_json()),
            });
        }
        Ok(Bundle {
            format: BUNDLE_FORMAT.to_owned(),
            actor: actor.to_owned(),
            from,
            to,
            manifest: Manifest {
                entries: entries
                    .iter()
                    .map(|e| PositionDigest {
                        position: e.position,
                        sha256: sha(&e.to_json()),
                    })
                    .collect(),
                charts: BTreeMap::from([(version.clone(), sha(&nquads))]),
                explanations,
            },
            entries,
            charts: BTreeMap::from([(version.clone(), nquads)]),
        })
    }
}
