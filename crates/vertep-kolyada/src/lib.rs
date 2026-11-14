//! Kolyada: verifiable role-based agent protocols.
//!
//! A *protocol* is one JSON-LD document in a restricted profile ([`document`]): roles, message
//! types with payload shapes ([`shape`]) and a choreography of states and transitions. Its
//! identity is the hash of its RDF graph ([`ProtocolId`]); each role runs a projection of it
//! ([`Protocol::project`]). Messages are signed before any transport seals them ([`envelope`]),
//! and an [`Instance`] judges each one with a pure conformance check, holding messages whose
//! dependencies have not arrived and keeping proof of every refusal ([`violation`]). Signed
//! [`offer`]s say which DIDs play which role.
//!
//! The crate depends on `vertep-rdf` for canonical hashing and on no interpreter, runtime, store
//! or transport: signing and verification enter through the [`Signer`] and [`Verifier`] traits.
//! See `tg.md/views/kolyada.md` for the architecture.
//!
//! @tg: implements:: [[decisions#A protocol is a role-based choreography and the interface is its projection]]
//! @tg: implements:: [[decisions#Choreographies are one global chart projected in a restricted subset]]
//! @tg: implements:: [[decisions#Protocols are identified by the hash of their graph]]
//! @tg: implements:: [[decisions#Protocol documents are JSON-LD in a restricted profile]]
//! @tg: implements:: [[decisions#Protocol messages are signed, then sealed]]
//! @tg: implements:: [[decisions#Out-of-order messages wait, and refusals come with proof]]
//! @tg: implements:: [[decisions#A protocol space is logical, sealed pairwise and hash-linked]] {confidence: partial, note: "the instance state machine; sealing and mailboxes live in vertep-federation"}
//! @tg: implements:: [[decisions#Role gives authority and the agent's policy decides at join]] {confidence: partial, note: "authority and joins; the agent policy lives in vertep-federation"}
//! @tg: implements:: [[decisions#The protocol core is a crate with one-way dependencies]]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod bridge;
pub mod document;
pub mod envelope;
pub mod instance;
pub mod offer;
pub mod projection;
pub mod shape;
pub mod violation;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use document::{
    KOLYADA_CONTEXT, Limits, Protocol, ProtocolError, ProtocolId, Transition, WellFormedError,
};
pub use envelope::{Body, Message, MessageHash, SignError, Signer, Verifier, VerifyError, control};
pub use instance::{
    Event, HistoryEntry, Instance, InstanceError, Reason, SendError, Status, Verdict,
};
pub use offer::{Offer, OfferBody, ServiceProtocol};
pub use projection::{Action, Interface, Projection, Step};
pub use shape::{Shape, ShapeError};
pub use violation::{Equivocation, Violation};
