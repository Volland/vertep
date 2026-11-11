//! The protocol seam between the runtime integration ([`crate::outbox`], [`crate::inbox`]) and
//! the relay protocols (DIDComm v2 with feature `didcomm`, TSP with feature `tsp`).
//!
//! A [`RelayProtocol`] seals an event envelope for a resolved recipient over one route and posts
//! it, and opens a picked-up message for a local actor into an [`OpenedMessage`] with its
//! verified origin. Everything around it (grants, retries, fallback, pickup, acknowledgement,
//! deduplication) is protocol-neutral.

use std::fmt;
use std::sync::Arc;

use crate::directory::{ResolvedAddress, Route, TransportKind};
use crate::envelope::RelayEnvelope;
use crate::error::CommunicationFailure;
use crate::identity::ActorIdentity;
use crate::keys::KeyManager;
use crate::net::{NetError, RelayNetwork};
use crate::origin::{OpenedMessage, Refusal};
use crate::resolver::DidResolver;
use crate::time::Timestamp;

/// A DID actor hosted on this node: its identity and the key manager holding its keys.
#[derive(Clone)]
pub struct LocalActor {
    /// The identity (DID, key names).
    pub identity: ActorIdentity,
    /// The key manager.
    pub km: Arc<dyn KeyManager>,
}

impl fmt::Debug for LocalActor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalActor")
            .field("did", &self.identity.did)
            .finish()
    }
}

/// Why one send attempt over one route failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The endpoint did not answer: try the next route, then retry later.
    Unreachable(String),
    /// The endpoint or a key no longer exists: re-resolve the DID once and retry.
    Stale(String),
    /// The recipient or a relay refused: the send fails.
    Refused(String),
    /// The message cannot be produced (missing keys, sealing failure): the send fails.
    Failed(CommunicationFailure),
    /// This route cannot carry this send (a TSP route for a sender without a DID): try the
    /// next route; no retry.
    Unsupported(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Unreachable(m) => write!(f, "unreachable: {m}"),
            SendError::Stale(m) => write!(f, "stale endpoint or key: {m}"),
            SendError::Refused(m) => write!(f, "refused: {m}"),
            SendError::Failed(m) => m.fmt(f),
            SendError::Unsupported(m) => write!(f, "route not usable: {m}"),
        }
    }
}

impl From<NetError> for SendError {
    fn from(e: NetError) -> Self {
        match e {
            NetError::Unreachable(m) => SendError::Unreachable(m),
            NetError::Gone(m) => SendError::Stale(m),
            NetError::Refused(m) => SendError::Refused(m),
        }
    }
}

impl SendError {
    /// The `error.communication` failure this becomes once no route or retry is left.
    pub fn to_failure(&self) -> CommunicationFailure {
        match self {
            SendError::Unreachable(m) | SendError::Stale(m) => {
                CommunicationFailure::Unreachable(m.clone())
            }
            SendError::Refused(m) => CommunicationFailure::Refused(m.clone()),
            SendError::Failed(f) => f.clone(),
            SendError::Unsupported(_) => CommunicationFailure::NoCompatibleEndpoint,
        }
    }
}

/// One send: who sends (`None` for a sender without a DID, which sends anonymously), to whom,
/// over which route, and the services it may use.
#[derive(Clone, Copy)]
pub struct SendContext<'a> {
    /// The sending DID actor, if the sender has a DID.
    pub from: Option<&'a LocalActor>,
    /// The resolved recipient.
    pub to: &'a ResolvedAddress,
    /// The route being tried.
    pub route: &'a Route,
    /// The event envelope.
    pub envelope: &'a RelayEnvelope,
    /// The network.
    pub net: &'a dyn RelayNetwork,
    /// The resolver, for routing keys and intermediaries.
    pub resolver: &'a dyn DidResolver,
}

impl fmt::Debug for SendContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SendContext")
            .field("from", &self.from.map(|a| a.identity.did.as_str()))
            .field("to", &self.to.did.as_str())
            .field("route", &self.route.service_id)
            .finish()
    }
}

/// A relay protocol: sealing and posting on the way out, opening on the way in.
pub trait RelayProtocol: Send + Sync {
    /// The transport this protocol implements.
    fn kind(&self) -> TransportKind;

    /// Seals the envelope for the recipient over the route and posts it.
    ///
    /// # Errors
    ///
    /// [`SendError`] classifying the failure for fallback and retry.
    fn send(&self, cx: &SendContext<'_>) -> Result<(), SendError>;

    /// Opens a picked-up message addressed to the local actor `to`.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] to record: undecryptable, forged origin or malformed.
    fn open(
        &self,
        to: &LocalActor,
        bytes: &[u8],
        resolver: &dyn DidResolver,
        now: Timestamp,
    ) -> Result<OpenedMessage, Refusal>;
}
