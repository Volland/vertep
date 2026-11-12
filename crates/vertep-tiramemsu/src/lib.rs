//! The default Vertep storage backend: journals and current-state graphs in tiramemsu, the
//! bitemporal, never-forget triple store.
//!
//! A [`TiramemsuShard`] is one tiramemsu file and implements both
//! [`vertep_store::JournalStore`] and [`vertep_store::GraphStore`], so an entry and the state it
//! causes commit in one transaction, state statements are derived beliefs whose support is the
//! entry that caused them, and the current-state graph can be read as of any past transaction.
//! A [`TiramemsuBackend`] is a directory of shard files, one writer each, with actors placed by
//! a hash of their address IRI.
//!
//! @tg: implements:: [[decisions#Two storage traits with tiramemsu as default]]
//! @tg: implements:: [[decisions#Fencing epochs on journal appends]] {confidence: partial, note: "epoch check per scope; leases are M4"}

mod backend;
mod convert;
mod shard;

pub use backend::{TiramemsuBackend, shard_file_name, shard_of};
pub use shard::{CAPABILITIES, TiramemsuShard, TiramemsuView, graph_capabilities};

/// The tiramemsu crate this backend is built on.
pub use tiramemsu;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
