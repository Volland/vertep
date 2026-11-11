//! Storage contracts for Vertep actors.
//!
//! Storage is two RDF-agnostic traits, so that no backend is asked for a guarantee it cannot
//! give:
//!
//! - [`JournalStore`]: per actor address, a linearizable, all-or-nothing append of journal
//!   entries at an expected position (compare-and-set), reads from a position, and an optional
//!   fencing token checked on append.
//! - [`GraphStore`]: RDF quads in, SPARQL 1.1 out, with a declared RDF 1.2 capability. No
//!   compare-and-set and no transactions across requests are required.
//!
//! Journal entries are RDF datasets with one named graph per entry ([`Entry`]). The crate also
//! ships the pieces every backend shares: the in-memory reference implementation
//! ([`MemoryStore`]), the projector that copies journals into an external graph store
//! ([`Projector`]), the RDF 1.1 reification fallback for annotations ([`reification`]), the
//! SPARQL text helpers ([`sparql`]) and, behind the `testkit` feature, the shared backend test
//! kit every backend must pass.
//!
//! @tg: implements:: [[decisions#Two storage traits with tiramemsu as default]] {confidence: partial}
//! @tg: implements:: [[decisions#Fencing epochs on journal appends]] {confidence: partial}

mod error;
mod graph;
mod journal;
mod memory;
mod projector;
pub mod reification;
pub mod sparql;
pub mod vocab;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use error::{AppendError, StoreError};
pub use graph::{GraphCapabilities, GraphStore, QueryResults, RdfCapability, Solutions};
pub use journal::{
    AppendMeta, Entry, FencingToken, JournalCapabilities, JournalRecord, JournalStore, Position,
    StateDelta, prepare_append,
};
pub use memory::MemoryStore;
pub use projector::{ProjectionReport, Projector};

/// The RDF model the contracts are written in (the `oxrdf` crate, with RDF 1.2 terms).
pub use oxrdf;
