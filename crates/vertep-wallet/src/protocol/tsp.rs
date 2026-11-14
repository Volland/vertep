//! The Vertep Credential Exchange over TSP (feature `tsp`, design D10): a [`Wire`] that seals
//! each exchange message to the peer's VID as a TSP application message, inside an established
//! TSP relationship, direct or routed through intermediaries (a mediator) that see only sealed
//! bytes.
//!
//! The first message to a peer without a relationship sends a relationship request and waits;
//! the peer's [`TspWire::open`] accepts requests, and the acceptance flushes the waiting
//! messages. Routed messages are nested sealed messages and need no relationship with the
//! intermediaries.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vertep_core::datamodel::{Event, EventType};
use vertep_federation::RelayEnvelope;
use vertep_federation::tsp::{Relationship, SealedTsp, TspError, TspReceived, TspSession, Vid};

use crate::error::WalletError;
use crate::exchange::Wire;
use crate::jose::{b64, unb64};

/// Resolves a DID to its TSP VID.
pub type ResolveVid = Arc<dyn Fn(&str) -> Result<Vid, TspError> + Send + Sync>;
/// Hands a sealed message to the transport, addressed to the DID that must open it next.
pub type Deliver = Arc<dyn Fn(&str, SealedTsp) -> Result<(), WalletError> + Send + Sync>;

/// The event name exchange messages travel under.
pub const EXCHANGE_EVENT: &str = "vertep.exchange";

fn tsp(e: TspError) -> WalletError {
    WalletError::Protocol(format!("TSP: {e}"))
}

/// An exchange message opened from TSP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The protocol IRI.
    pub protocol: String,
    /// The message id.
    pub msg: String,
    /// The encoded exchange message.
    pub bytes: Vec<u8>,
    /// The sender's VID, verified by its TSP signature.
    pub sender: String,
}

/// The TSP wire of one wallet.
// @tg: implements:: [[openspec:credential-exchange#Native Exchange Over TSP]]
pub struct TspWire {
    session: Arc<TspSession>,
    resolve: ResolveVid,
    deliver: Deliver,
    routes: Mutex<BTreeMap<String, Vec<String>>>,
    waiting: Mutex<BTreeMap<String, Vec<RelayEnvelope>>>,
}

impl std::fmt::Debug for TspWire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TspWire").finish_non_exhaustive()
    }
}

impl TspWire {
    /// A wire for `session`, resolving peers with `resolve` and sending through `deliver`.
    pub fn new(session: Arc<TspSession>, resolve: ResolveVid, deliver: Deliver) -> Self {
        TspWire {
            session,
            resolve,
            deliver,
            routes: Mutex::default(),
            waiting: Mutex::default(),
        }
    }

    /// Routes messages to `peer` through the intermediaries `hops` (DIDs), in order.
    pub fn route(&self, peer: &str, hops: &[&str]) {
        if let Ok(mut r) = self.routes.lock() {
            r.insert(peer.to_owned(), hops.iter().map(|h| (*h).to_owned()).collect());
        }
    }

    fn seal_direct(&self, to: &str, peer: &Vid, env: &RelayEnvelope) -> Result<(), WalletError> {
        let sealed = self.session.seal_event(peer, env).map_err(tsp)?;
        (self.deliver)(to, sealed)
    }

    /// Opens a TSP message addressed to this wallet: an exchange message to hand to
    /// [`crate::exchange::Exchanges::receive`], or `None` for relationship messages, which it
    /// answers (accepting requests) or completes (flushing waiting messages).
    ///
    /// # Errors
    ///
    /// [`WalletError::Protocol`] for messages that do not open or are not exchange messages.
    pub fn open(&self, bytes: &mut [u8]) -> Result<Option<Opened>, WalletError> {
        let resolve = self.resolve.clone();
        match self.session.open(bytes, &|id| resolve(id)).map_err(tsp)? {
            TspReceived::Event(m) => {
                if m.envelope.name != EXCHANGE_EVENT {
                    return Err(WalletError::Unmapped(m.envelope.name.clone()));
                }
                let data = vertep_agent::json::to_json(&m.envelope.data.0);
                let protocol = data
                    .get("protocol")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Malformed("TSP exchange without protocol".into()))?
                    .to_owned();
                let body = data
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or_else(|| WalletError::Malformed("TSP exchange without message".into()))?;
                Ok(Some(Opened {
                    protocol,
                    msg: m.envelope.message_id.clone(),
                    bytes: unb64(body)?,
                    sender: m.origin.sender.clone().unwrap_or_default(),
                }))
            }
            TspReceived::RelationshipRequested { sender, thread_id } => {
                let peer = (self.resolve)(&sender).map_err(tsp)?;
                let reply = self.session.accept_relationship(&peer, thread_id).map_err(tsp)?;
                (self.deliver)(&sender, reply)?;
                Ok(None)
            }
            TspReceived::RelationshipAccepted { sender } => {
                let peer = (self.resolve)(&sender).map_err(tsp)?;
                let waiting = self
                    .waiting
                    .lock()
                    .map(|mut w| w.remove(&sender).unwrap_or_default())
                    .unwrap_or_default();
                for env in waiting {
                    self.seal_direct(&sender, &peer, &env)?;
                }
                Ok(None)
            }
            TspReceived::RelationshipCancelled { sender } => {
                if let Ok(mut w) = self.waiting.lock() {
                    w.remove(&sender);
                }
                Ok(None)
            }
        }
    }
}

impl Wire for TspWire {
    fn send(
        &self,
        protocol: &str,
        _from: &str,
        to: &str,
        msg_id: &str,
        bytes: &[u8],
    ) -> Result<(), WalletError> {
        let event = Event {
            name: EXCHANGE_EVENT.to_owned(),
            type_: EventType::External,
            data: vertep_agent::json::from_json(&json!({"protocol": protocol, "message": b64(bytes)})),
            ..Event::default()
        };
        let env = RelayEnvelope::from_event(msg_id, &event);
        let peer = (self.resolve)(to).map_err(tsp)?;
        let hops = self
            .routes
            .lock()
            .ok()
            .and_then(|r| r.get(to).cloned())
            .unwrap_or_default();
        if !hops.is_empty() {
            let vids = hops
                .iter()
                .map(|h| (self.resolve)(h))
                .collect::<Result<Vec<_>, _>>()
                .map_err(tsp)?;
            let sealed = self.session.seal_routed(&peer, &vids, &env).map_err(tsp)?;
            return (self.deliver)(&hops[0], sealed);
        }
        match self.session.relationship(to) {
            Some(Relationship::Established) => self.seal_direct(to, &peer, &env),
            Some(Relationship::Requested(_)) => {
                if let Ok(mut w) = self.waiting.lock() {
                    w.entry(to.to_owned()).or_default().push(env);
                }
                Ok(())
            }
            _ => {
                if let Ok(mut w) = self.waiting.lock() {
                    w.entry(to.to_owned()).or_default().push(env);
                }
                let rfi = self.session.request_relationship(&peer).map_err(tsp)?;
                (self.deliver)(to, rfi)
            }
        }
    }
}
