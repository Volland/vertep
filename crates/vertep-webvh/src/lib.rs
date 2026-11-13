//! `did:webvh` for Vertep (milestone M8, OpenSpec change `m8-webvh`).
//!
//! Vertep resolves `did:webvh` through `vertep-federation`; this crate writes and hosts it:
//!
//! - [`author`]: create, update, rotate (with pre-rotation) and deactivate DID logs, signing
//!   every entry through a [`KeyManager`](vertep_federation::KeyManager) ([`KmSigner`]);
//! - [`document`]: DID documents from Vertep identities and durable-object custody declarations;
//! - [`log`]: a DID log with its witness proofs, verified with the `did:webvh` resolver;
//! - [`publish`]: the `did:webvh` 1.0 file layout and the directory, git and command publishers,
//!   each of which verifies before it writes;
//! - [`fetch`]: fetchers that map `https://<domain>/` onto directories or local servers;
//! - [`witness`] (feature `custody`): custodians as witnesses of their object's log, and the
//!   custody [`DocumentPublisher`](vertep_objects::custody::DocumentPublisher);
//! - [`server`] (feature `server`): the log server behind `vt webvh serve`.
//!
//! Log mechanics (SCIDs, entry hashes, Data Integrity proofs, pre-rotation, the witness proof
//! format) are the pinned DIF `didwebvh-rs`.
//!
//! See `tg.md/views/durable-objects.md` for the architecture.
//!
//! @tg: implements:: [[decisions#did:webvh authoring builds on didwebvh-rs]]
//! @tg: implements:: [[decisions#did:webvh entries are signed through the key manager]]
//! @tg: implements:: [[decisions#Log publishers verify before they write]]

pub mod author;
pub mod document;
pub mod error;
pub mod fetch;
pub mod log;
pub mod publish;
#[cfg(feature = "server")]
pub mod server;
pub mod signer;
#[cfg(feature = "custody")]
pub mod witness;

pub use author::{Author, Clock, CreateOptions, UpdateKeys, Witnessing};
pub use error::WebvhError;
pub use fetch::{DirectoryFetcher, RoutedFetcher};
pub use log::{DidLog, Verified};
pub use publish::{
    CommandPublisher, DirectoryPublisher, GitPublisher, LogPublisher, Published, did_files, log_dir,
};
pub use signer::{KmSigner, did_key, key_hash};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
