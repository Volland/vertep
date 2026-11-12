//! Event-sourced actor runtime for Vertep.
//!
//! A chart becomes a durable actor: addressed by an IRI, fed through a mailbox, run one SCXML
//! macrostep per turn by the pure `vertep_core::session::step`, and made durable by appending
//! one journal entry per turn to a [`vertep_store::JournalStore`] with compare-and-set. The
//! runtime is an imperative shell around that pure core: it owns time ([`shell::Clock`]),
//! background work ([`shell::Spawn`]) and waiting ([`shell::Sleep`]), all injectable, so tests
//! run on a virtual clock.
//!
//! - [`node`]: the scheduler of one process: virtual actors (activated on message, deactivated
//!   when idle, at most one activation per node), invoked actors (`<invoke>` children with
//!   `vt:restart` supervision), stateless worker pools, the transactional outbox, durable
//!   timers, snapshots, replay, migrations, forks and time travel.
//! - [`machine`]: the deterministic macrostep used by live turns and replay alike.
//! - [`journal`]: journal entries as RDF datasets in `vt:` terms.
//! - [`mailbox`], [`transport`]: pluggable mailboxes (in-memory, store-backed), transports
//!   registered as SCXML Event I/O Processors, and the directory of resolvers.
//! - [`actor`]: the actor contract and its chart, function and stream adapters.
//! - [`chart`]: chart versions, the registry, `vt:` extension lowering and migrations.
//! - [`ontology`]: the runtime ontology module and its SHACL shapes.
//! - [`placement`]: the hook a cluster uses to place virtual actors and workers on nodes.
//! - [`websocket`]: the WebSocket Event I/O Processor's frames and links, and its server end
//!   (feature `websocket`).
//!
//! @tg: implements:: [[decisions#SCXML-native actors with vt extensions]]
//! @tg: implements:: [[decisions#IRI identity and lifecycle classes]]
//! @tg: implements:: [[decisions#Identity, mailbox and transport layers]] {confidence: partial, note: "local transport only; DID and relay mailboxes are M5"}
//! @tg: implements:: [[decisions#Event-sourced actors]]
//! @tg: implements:: [[decisions#Content-addressed chart versions]]
//! @tg: implements:: [[decisions#No-std core and tokio shell]]
//! @tg: implements:: [[decisions#Non-blocking request and deferred events]]
//! @tg: implements:: [[decisions#State changes travel in the append]]
//! @tg: implements:: [[decisions#Fencing epochs on journal appends]] {confidence: partial, note: "the configured token is passed on every append; leases are M4"}

pub mod actor;
pub mod address;
pub mod chart;
pub mod journal;
pub mod json;
pub mod machine;
pub mod mailbox;
pub mod node;
pub mod ontology;
pub mod placement;
pub mod shell;
pub mod storage;
pub mod transport;
pub mod vocab;
pub mod websocket;

pub use actor::{
    Actor, ActorFactory, ActorFailure, ActorFn, ActorInput, ActorStart, ChartActor, FnActor,
    OutTarget, Outgoing, StreamActor, Turn, TurnOutput,
};
pub use address::{Address, AddressError};
pub use chart::{ChartInfo, ChartRegistry, Migration, RegisterError, UnhandledPolicy};
pub use journal::{EntryBody, EntryData, TimerRecord};
pub use mailbox::{Envelope, Mailbox, MailboxFactory, MemoryMailboxes, StoreMailboxes};
pub use node::{
    ActivationError, ActorState, AskError, CrashPoint, DeadLetter, DeliveryError, MigrateError,
    Node, NodeBuilder, NodeConfig, NodeHandle, NodeStats, RemoveError,
};
pub use placement::{Placement, Route};
pub use shell::{Clock, Shell, Sleep, Spawn, VirtualClock};
pub use storage::{SplitStorage, Storage};
pub use transport::{LocalTransport, Resolver, Transport};

/// The chart RDF crate (version IRIs).
pub use vertep_rdf;
/// The storage contracts.
pub use vertep_store;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
