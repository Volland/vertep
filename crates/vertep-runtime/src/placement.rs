//! The placement hook of a clustered node.
//!
//! A single node hosts every virtual actor whose chart type it knows. In a cluster, each
//! virtual actor lives on the node that leases its shard, so the node asks a [`Placement`]
//! where an actor lives before it delivers a message to it or activates it. Without a
//! placement (the default), the node behaves exactly as a single node.
//!
//! The answer is a [`Route`]:
//!
//! - [`Route::Local`]: this node hosts the actor now; deliver and activate as usual.
//! - [`Route::Remote`]: another node hosts it; the message goes into the given mailbox, which
//!   carries it there. Messages already waiting in the local mailbox are moved there too, and
//!   an activation on this node is deactivated first.
//! - [`Route::Hold`]: no node may host it now (its shard is unowned or its owner is gone); the
//!   message waits in the local mailbox and the actor is not activated.
//!
//! [`Placement::route`] sees every message to a virtual actor or a stateless worker before it
//! is delivered, so a cluster can also balance worker requests and observe their replies.
//!
//! @tg: implements:: [[decisions#Placement is a runtime hook with three answers]]

use std::sync::Arc;

use crate::address::Address;
use crate::mailbox::{Envelope, Mailbox};

/// Where an actor lives now.
#[derive(Clone)]
pub enum Route {
    /// On this node.
    Local,
    /// On another node, reached through this mailbox.
    Remote(Arc<dyn Mailbox>),
    /// Nowhere yet: keep the message here and do not activate the actor.
    Hold,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Local => f.write_str("Local"),
            Route::Remote(_) => f.write_str("Remote"),
            Route::Hold => f.write_str("Hold"),
        }
    }
}

/// Decides where virtual actors and stateless workers live in a cluster.
pub trait Placement: Send + Sync {
    /// Where the actor at `to` lives, for delivering `env` (`None` when the node asks before
    /// activating `to`, with no message in hand). `virtual_actor` is true when this node knows
    /// `to` as a virtual actor (its chart type is registered or it has a journal here); the
    /// node also asks for stateless workers and for addresses it does not know, which a
    /// cluster may host elsewhere. [`Route::Local`] for an address the node does not know
    /// sends it on to the directory.
    fn route(&self, to: &Address, virtual_actor: bool, env: Option<&Envelope>) -> Route;
}
