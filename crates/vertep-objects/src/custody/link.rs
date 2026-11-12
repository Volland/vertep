//! How a host reaches a custodian: the [`CustodyTransport`] seam, an in-process transport
//! with fault injection, and the adapter that lets the cluster's suffix shipping
//! ([`vertep_cluster::ship_suffix`]) ship custody batches.

use core::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use vertep_cluster::{ReplicaError, ReplicaLink};
use vertep_store::{Entry, Position};

use super::config::{CustodyConfig, Role};
use super::replica::Replica;
use super::wire::{
    AppendBatch, ContentKey, CustodyRequest, CustodyResponse, KeyWrap, WireEntry, seal_for_keeper,
};
use crate::chain::EntrySigner;

/// Why a request did not get an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError(pub String);

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "custody link: {}", self.0)
    }
}

impl std::error::Error for LinkError {}

/// Carries custody requests to one custodian and brings back its answer.
pub trait CustodyTransport: Send + Sync {
    /// The custodian's DID.
    fn custodian(&self) -> &str;
    /// Sends `request` and waits for the answer.
    ///
    /// # Errors
    ///
    /// [`LinkError`] when the custodian cannot be reached or answers garbage.
    fn call(&self, request: &CustodyRequest) -> Result<CustodyResponse, LinkError>;
}

/// A custodian in the same process (tests, and a host's own replica), with a switch that
/// disconnects it.
pub struct LocalTransport {
    replica: Arc<Replica>,
    up: AtomicBool,
    appends: AtomicBool,
    log: Mutex<Vec<String>>,
}

impl fmt::Debug for LocalTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalTransport")
            .field("custodian", &self.replica.custodian())
            .finish_non_exhaustive()
    }
}

impl LocalTransport {
    /// The transport to `replica`.
    pub fn new(replica: Arc<Replica>) -> Self {
        LocalTransport {
            replica,
            up: AtomicBool::new(true),
            appends: AtomicBool::new(true),
            log: Mutex::new(Vec::new()),
        }
    }

    /// Connects or disconnects (fault injection).
    pub fn set_up(&self, up: bool) {
        self.up.store(up, Ordering::SeqCst);
    }

    /// Lets appends through or drops them, while promises and reads still pass (fault
    /// injection: a custodian that promised and then became unreachable).
    pub fn set_appends(&self, through: bool) {
        self.appends.store(through, Ordering::SeqCst);
    }

    /// The replica.
    pub fn replica(&self) -> &Arc<Replica> {
        &self.replica
    }

    /// The JSON of every request carried, for inspection.
    pub fn carried(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl CustodyTransport for LocalTransport {
    fn custodian(&self) -> &str {
        self.replica.custodian()
    }

    fn call(&self, request: &CustodyRequest) -> Result<CustodyResponse, LinkError> {
        if !self.up.load(Ordering::SeqCst) {
            return Err(LinkError(format!("{} is disconnected", self.custodian())));
        }
        if matches!(request, CustodyRequest::Append(_)) && !self.appends.load(Ordering::SeqCst) {
            return Err(LinkError(format!("{} drops appends", self.custodian())));
        }
        // Round-trip through JSON so in-process tests exercise the wire form.
        let text = serde_json::to_string(request).map_err(|e| LinkError(e.to_string()))?;
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(text.clone());
        let request: CustodyRequest =
            serde_json::from_str(&text).map_err(|e| LinkError(e.to_string()))?;
        let answer = self.replica.handle(&request);
        let text = serde_json::to_string(&answer).map_err(|e| LinkError(e.to_string()))?;
        serde_json::from_str(&text).map_err(|e| LinkError(e.to_string()))
    }
}

/// What the writer ships: clear entries, wrapped per custodian by [`CustodianLink`].
#[derive(Clone, Debug)]
pub struct WriterBatch {
    /// The epoch.
    pub epoch: u64,
    /// The position the entries follow.
    pub after: Position,
    /// Sealed clear entries.
    pub entries: Vec<Entry>,
    /// The effective configuration shipped.
    pub config: CustodyConfig,
    /// The pending configuration of a change in progress.
    pub next: Option<CustodyConfig>,
    /// The key wraps shipped.
    pub keys: Vec<KeyWrap>,
}

/// One custodian as the writer ships to it: encrypts entries for keepers, signs the batch and
/// maps the custodian's answers onto the replication protocol's outcomes.
pub struct CustodianLink {
    /// The object.
    pub object: String,
    /// The custodian's role.
    pub role: Role,
    /// The transport.
    pub transport: Arc<dyn CustodyTransport>,
    /// The writer.
    pub signer: Arc<dyn EntrySigner>,
    /// The content key keepers' entries are encrypted under.
    pub content_key: Option<ContentKey>,
}

impl fmt::Debug for CustodianLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustodianLink")
            .field("custodian", &self.transport.custodian())
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl ReplicaLink<WriterBatch> for CustodianLink {
    fn replica(&self) -> String {
        self.transport.custodian().to_owned()
    }

    fn replicate(&self, batch: &WriterBatch) -> Result<Position, ReplicaError> {
        let entries: Vec<WireEntry> = match self.role {
            Role::Host => batch.entries.iter().map(WireEntry::clear).collect(),
            Role::Keeper => {
                let key = self
                    .content_key
                    .as_ref()
                    .ok_or_else(|| ReplicaError::Store("no content key for keepers".into()))?;
                batch
                    .entries
                    .iter()
                    .map(|e| {
                        seal_for_keeper(e, key, self.signer.as_ref())
                            .map(WireEntry::Sealed)
                            .map_err(|e| ReplicaError::Store(e.to_string()))
                    })
                    .collect::<Result<_, _>>()?
            }
        };
        let request = AppendBatch {
            object: self.object.clone(),
            epoch: batch.epoch,
            writer: self.signer.writer().to_owned(),
            after: batch.after.get(),
            entries,
            config: batch.config.clone(),
            next: batch.next.clone(),
            keys: batch.keys.clone(),
            signature: String::new(),
        }
        .sign(self.signer.as_ref())
        .map_err(|e| ReplicaError::Store(e.to_string()))?;
        match self
            .transport
            .call(&CustodyRequest::Append(request))
            .map_err(|e| ReplicaError::Unreachable(e.0))?
        {
            CustodyResponse::Ack { head, .. } => Ok(Position::new(head)),
            CustodyResponse::Behind { head } => Err(ReplicaError::Behind {
                head: Position::new(head),
            }),
            CustodyResponse::Refused { highest, reason } if highest > batch.epoch => {
                let _ = reason;
                Err(ReplicaError::Fenced {
                    epoch: batch.epoch,
                    highest,
                })
            }
            CustodyResponse::Refused { reason, .. } => Err(ReplicaError::Store(reason)),
            CustodyResponse::Diverged { position, .. } => {
                Err(ReplicaError::Diverged(Position::new(position)))
            }
            CustodyResponse::Failed { reason } => Err(ReplicaError::Store(reason)),
            other => Err(ReplicaError::Store(format!("unexpected answer {other:?}"))),
        }
    }
}
