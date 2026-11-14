//! Vertep: SCXML statecharts as durable, event-sourced actors, with charts and state as RDF.
//!
//! This facade re-exports the Vertep crates behind cargo features:
//!
//! | feature | crate | what |
//! |---|---|---|
//! | (always) | [`core`] | chart IR, validation, the pure `step` kernel |
//! | (always) | [`store`] | `JournalStore`, `GraphStore`, the in-memory store |
//! | (always) | [`rdf`] | chart projection, version IRIs, SHACL |
//! | `runtime` (default) | [`runtime`] | the actor runtime on tokio |
//! | `tiramemsu` (default) | [`tiramemsu`] | the default storage backend |
//! | `scxml` (default) | [`scxml`] | the SCXML reader and writer |
//! | `ecmascript` | [`ecmascript`] | the ECMAScript datamodel |
//! | `federation` | `federation` | DID identities, grants, DIDComm v2 and TSP relays |
//! | `objects` | `objects` | durable objects: verifiable journals, custody over TSP |
//!
//! Start with [`prelude`] and `examples/quickstart.rs`: one virtual actor on tiramemsu, a
//! durable delayed send, and a time-travel query.

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub use vertep_core as core;
pub use vertep_rdf as rdf;
pub use vertep_store as store;

pub use vertep_store::{GraphStore, JournalStore, MemoryStore};

#[cfg(feature = "runtime")]
pub use vertep_runtime as runtime;

#[cfg(feature = "tiramemsu")]
pub use vertep_tiramemsu as tiramemsu;

#[cfg(feature = "scxml")]
pub use vertep_scxml as scxml;

#[cfg(feature = "ecmascript")]
pub use vertep_dm_ecma as ecmascript;

#[cfg(feature = "agent")]
pub use vertep_agent as agent;

#[cfg(feature = "federation")]
pub use vertep_federation as federation;

#[cfg(feature = "cluster")]
pub use vertep_cluster as cluster;

#[cfg(feature = "objects")]
pub use vertep_objects as objects;

#[cfg(feature = "wallet")]
pub use vertep_wallet as wallet;

/// The names most programs need.
pub mod prelude {
    pub use vertep_core::datamodel::{Event, EventType, Value};
    pub use vertep_core::ir::Chart;
    pub use vertep_store::{GraphStore, JournalStore, MemoryStore, Position};

    #[cfg(feature = "runtime")]
    pub use vertep_runtime::{
        Actor, ActorState, Address, Envelope, FnActor, Migration, Node, NodeConfig, NodeHandle,
        Shell, Storage, StoreMailboxes, VirtualClock,
    };

    #[cfg(feature = "tiramemsu")]
    pub use vertep_tiramemsu::{TiramemsuBackend, TiramemsuShard};
}
