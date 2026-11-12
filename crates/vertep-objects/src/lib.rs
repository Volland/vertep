//! Durable objects for Vertep (milestone M7, OpenSpec change `m7-durable-objects`).
//!
//! A durable object is a virtual actor with a `did:webvh` address whose journal is
//! hash-chained and signed ([`chain`]), kept by a custodian set named in its DID document
//! ([`custody`]), and written by one host per custody epoch, won by promises from a majority of
//! the custodians. The custody journal ([`custody::CustodyJournal`]) is an ordinary
//! `JournalStore`, so the runtime's virtual-actor machinery runs a durable object unchanged and
//! produces effects only after an append has been committed by a quorum.
//!
//! See `tg.md/views/durable-objects.md` for the architecture.
//!
//! @tg: implements:: [[decisions#Durable objects are DID virtual actors with verifiable journals]]
//! @tg: implements:: [[decisions#Journal entries are hash-chained and host-signed]]
//! @tg: implements:: [[decisions#Custody epochs are won by quorum promises]]

pub mod chain;
pub mod custody;
pub mod identity;
pub mod runtime;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod vocab;

pub use chain::{
    ChainError, ChainHead, ChainLink, Digest, EntrySigner, FailureReason, HostKeys,
    KeyManagerSigner, SealedLink, VerifyFailure, content_hash, read_link, seal, verify_chain,
    verify_entry,
};
pub use identity::{DurableObjectId, IdentityError};
pub use runtime::{ObjectStorage, serve_object};
