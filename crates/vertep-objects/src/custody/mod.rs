//! Decentralised custody of durable objects: custodian sets in the DID document, epochs won
//! by quorum promises, replication of the verifiable journal, encrypted keeper replicas,
//! handoff, failover, equivocation evidence and custodian-set changes.

pub mod config;
pub mod history;
pub mod host;
pub mod keys;
pub mod link;
pub mod replica;
#[cfg(feature = "tsp")]
pub mod tsp_link;
pub mod wire;

pub use history::History;
pub use config::{ConfigError, Custodian, CustodyConfig, Role};
pub use host::{
    Backoff, CustodyError, CustodyHost, CustodyStore, DocumentPublisher, Elected, EpochRecord,
    ReconfigurationRequest, RecordingPublisher,
};
pub use keys::{InstalledObjectKeys, ObjectKeyShare, ObjectKeys};
pub use link::{CustodianLink, CustodyTransport, LinkError, LocalTransport, WriterBatch};
pub use replica::{EvidenceSink, Replica};
pub use wire::{
    AppendBatch, ContentKey, CustodyRequest, CustodyResponse, EpochPromise, Evidence, KeeperEntry,
    KeyDirectory, KeyWrap, ResolverKeyDirectory, StaticKeyDirectory, WireEntry,
};
