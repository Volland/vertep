//! A TSP mailbox mediator for Vertep (milestone M7, OpenSpec capability `tsp-mediator`).
//!
//! The ToIP Trust Spanning Protocol (Release 1.0) routes messages through intermediaries but
//! defines no store-and-forward: it "provides no delivery guarantee". This crate is a
//! mediator that fills the gap without trusting anyone with content:
//!
//! - **Routing** ([`Mediator`]): a TSP 1.0 routed-mode intermediary. It opens only its own layer,
//!   forwards middle hops and, at the last hop, delivers the still-sealed nested message to the
//!   recipient, pushes it on a live session, or stores it.
//! - **Mailboxes** ([`store::MailboxStore`]): durable, event-sourced journals per recipient VID
//!   in any Vertep journal store (tiramemsu shard files by default), accepted from the previous
//!   hop only after the append, deleted only on the recipient's acknowledgement, with quotas and
//!   retention.
//! - **Pickup**: the versioned mailbox protocol of `vertep_federation::tsp_mailbox`, carried
//!   inside TSP over the relationship between a VID and the mediator, and a live session that
//!   pushes messages as they arrive.
//! - **Server** (feature `server`): HTTP and WebSocket endpoints on axum, the `vt-mediator`
//!   binary, an HTTP [`RelayNetwork`](vertep_federation::RelayNetwork) and a WebSocket
//!   [`LiveNetwork`](vertep_federation::LiveNetwork) for clients.
//!
//! See `tg.md/views/distribution.md` (TSP mediator) for the architecture.
//!
//! @tg: implements:: [[decisions#Vertep ships its own TSP mediator]]
//! @tg: implements:: [[decisions#TSP pickup is a versioned protocol inside TSP]]
//! @tg: implements:: [[decisions#The mediator stores opaque sealed bytes in its own journal]]
//! @tg: implements:: [[decisions#Live delivery is a WebSocket binding of pickup]]

pub mod mediator;
pub mod store;

#[cfg(feature = "server")]
pub mod config;
#[cfg(feature = "server")]
pub mod http;
#[cfg(feature = "server")]
pub mod server;

pub use mediator::{
    Limits, MailboxMetrics, Mediator, MediatorError, MediatorOptions, Metrics, RegistrationPolicy,
};
pub use store::{MailboxStore, MailboxStoreError};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
