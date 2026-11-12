//! The WebSocket Event I/O Processor: the frames both ends speak and the delivery state of one
//! peer link, shared by the browser shell (`vertep-wasm`) and the server endpoint
//! ([`server`], feature `websocket`).
//!
//! Frames are JSON text messages tagged by `type`:
//!
//! - `hello` (client to server, first frame): the client's node IRI, the credential its
//!   provider supplied, and the IRI prefixes of the actors it hosts.
//! - `welcome` or `refused` (server to client): the connection is accepted, or its credential
//!   is refused (the server then closes it).
//! - `message`: a Vertep envelope: message id, target IRI, origin IRI, event name, payload,
//!   send id and correlation id.
//! - `ack`: the receiver accepted the message with that id into a mailbox.
//!
//! Delivery is at least once. A [`Link`] keeps every message it sent until the peer
//! acknowledges it and sends all of them again after a reconnect; receivers deduplicate by
//! message id (the runtime journals message ids and drops a duplicate, see
//! [[openspec:event-journal#Message Deduplication]]).
//!
//! The runtime's transactional outbox holds a send until delivery succeeds: a send offered to a
//! [`LinkMailbox`] is refused with "awaiting acknowledgement" until the peer's `ack` arrives, so
//! the outbox retries it and its durable outbox mark passes it only once it is acknowledged.
//! That makes the redelivery survive a crash of the sender, too.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::channel::mpsc::UnboundedSender;
use serde::{Deserialize, Serialize};
use vertep_core::datamodel::{Event, EventType};

use crate::address::Address;
use crate::json::JsonValue;
use crate::mailbox::{Envelope, Mailbox, MailboxError};
use crate::node::NodeHandle;
use crate::transport::{Resolver, Transport, TransportError};

#[cfg(feature = "websocket")]
pub mod server;

/// The type IRI of the WebSocket Event I/O Processor (alias `websocket`). Events that arrive
/// over WebSocket carry it as `_event.origintype`.
pub const WEBSOCKET_PROCESSOR: &str = "https://w3id.org/vertep/vt#WebSocketEventProcessor";

/// The dead-letter reason of a send whose connection credential was refused.
pub const REFUSED_REASON: &str = "transport-refused";

/// A Vertep envelope on the wire.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// The message id, the receiver's deduplication key.
    pub id: String,
    /// The receiver's IRI.
    pub target: String,
    /// The sender's IRI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The event name.
    pub event: String,
    /// The payload, in the runtime's lossless JSON encoding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<JsonValue>,
    /// The send id (`_event.sendid`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sendid: Option<String>,
    /// A request's correlation id, or the one a reply answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation: Option<String>,
}

impl Message {
    /// The wire form of `env`.
    pub fn from_envelope(env: &Envelope) -> Self {
        Message {
            id: env.message_id.clone(),
            target: env.to.clone(),
            origin: env.event.origin.clone(),
            event: env.event.name.clone(),
            data: Some(JsonValue(env.event.data.clone())),
            sendid: env.event.sendid.clone(),
            correlation: env.correlation.clone(),
        }
    }

    /// The envelope a receiver delivers: an external event whose origin type names this
    /// processor and whose origin is the sender's IRI.
    pub fn into_envelope(self) -> Envelope {
        let mut event = Event::named(self.event, EventType::External);
        event.origin = self.origin;
        event.origintype = Some(WEBSOCKET_PROCESSOR.to_owned());
        event.sendid = self.sendid;
        if let Some(d) = self.data {
            event.data = d.0;
        }
        Envelope {
            message_id: self.id,
            to: self.target,
            event,
            correlation: self.correlation,
            timer: None,
        }
    }
}

/// One frame of the protocol.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Frame {
    /// The client's first frame.
    Hello {
        /// The client node's IRI, stable for the life of its actor system.
        node: String,
        /// The credential the client's provider supplied.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential: Option<String>,
        /// IRI prefixes of the actors the client hosts.
        #[serde(default)]
        hosts: Vec<String>,
    },
    /// The server accepted the connection.
    Welcome {
        /// The server node's IRI.
        node: String,
        /// IRI prefixes of the actors the server hosts.
        #[serde(default)]
        hosts: Vec<String>,
    },
    /// The server refused the credential and closes the connection.
    Refused {
        /// Why.
        reason: String,
    },
    /// A message.
    Message(Message),
    /// The receiver accepted message `id`.
    Ack {
        /// The acknowledged message id.
        id: String,
    },
}

impl Frame {
    /// The frame as JSON text.
    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parses a frame.
    ///
    /// # Errors
    ///
    /// A message for text that is not a frame.
    pub fn parse(text: &str) -> Result<Frame, String> {
        serde_json::from_str(text).map_err(|e| format!("not a Vertep WebSocket frame: {e}"))
    }
}

/// What became of a message offered to a [`Link`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Offer {
    /// The peer acknowledged it.
    Delivered,
    /// It is sent (or waits for a connection) and not acknowledged yet.
    Pending,
    /// The peer refused the connection credential.
    Refused(String),
}

/// What the receiving side of a [`Link`] should do with a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Inbound {
    /// Deliver this envelope, then [`Link::ack`] it.
    Deliver(Envelope),
    /// The peer acknowledged message `id`; `watched` when [`Link::watch`] asked to hear of it.
    Acked {
        /// The message id.
        id: String,
        /// Whether someone waits for this acknowledgement.
        watched: bool,
    },
    /// A handshake frame (`hello`, `welcome`); the connection code handles it.
    Handshake(Frame),
    /// The credential was refused: the watched messages that were pending, now failed.
    Refused {
        /// Why.
        reason: String,
        /// The watched message ids that were pending.
        failed: Vec<String>,
    },
}

#[derive(Debug)]
struct Pending {
    env: Envelope,
    sent_on: Option<u64>,
    order: u64,
}

#[derive(Default)]
struct State {
    connection: Option<(u64, UnboundedSender<String>)>,
    pending: BTreeMap<String, Pending>,
    acked: BTreeSet<String>,
    watched: BTreeSet<String>,
    settled: Vec<(String, Result<(), String>)>,
    refused: Option<String>,
    next_order: u64,
    routes: BTreeSet<String>,
}

/// The delivery state of one peer: the unacknowledged messages, the current connection, and
/// the acknowledgements received. It outlives connections, so a reconnect sends every pending
/// message again. It is `Send + Sync`; the I/O lives in the connection code, which hands the
/// link a channel of outgoing frame texts on connect.
// @tg: implements:: [[openspec:wasm-bindings#Server Interop Over WebSocket]] {confidence: partial, note: "envelopes, acknowledgements and redelivery, shared by both ends"}
#[derive(Default)]
pub struct Link {
    state: Mutex<State>,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("Link")
            .field("connected", &s.connection.as_ref().map(|c| c.0))
            .field("pending", &s.pending.len())
            .field("refused", &s.refused)
            .finish()
    }
}

impl Link {
    /// A link with no connection yet.
    pub fn new() -> Self {
        Link::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn send_pending(s: &mut State, id: &str) {
        let Some((generation, tx)) = s.connection.clone() else {
            return;
        };
        if let Some(p) = s.pending.get_mut(id)
            && p.sent_on != Some(generation)
        {
            p.sent_on = Some(generation);
            let _ = tx.unbounded_send(Frame::Message(Message::from_envelope(&p.env)).to_text());
        }
    }

    /// Offers `env` for delivery: sends it if connected and not yet sent on this connection,
    /// and reports whether the peer has acknowledged it. Offering the same message id again is
    /// how the outbox retries; it is sent once per connection.
    pub fn offer(&self, env: Envelope) -> Offer {
        let mut s = self.lock();
        if s.acked.remove(&env.message_id) {
            return Offer::Delivered;
        }
        if let Some(r) = &s.refused {
            return Offer::Refused(r.clone());
        }
        let id = env.message_id.clone();
        if !s.pending.contains_key(&id) {
            let order = s.next_order;
            s.next_order += 1;
            s.pending.insert(
                id.clone(),
                Pending {
                    env,
                    sent_on: None,
                    order,
                },
            );
        }
        Link::send_pending(&mut s, &id);
        Offer::Pending
    }

    /// Asks to hear of the acknowledgement (or refusal) of message `id` through
    /// [`Link::take_settled`] instead of a later [`Link::offer`].
    pub fn watch(&self, id: &str) {
        let mut s = self.lock();
        if s.acked.remove(id) {
            s.settled.push((id.to_owned(), Ok(())));
        } else {
            s.watched.insert(id.to_owned());
        }
    }

    /// The watched messages acknowledged or failed since the last call.
    pub fn take_settled(&self) -> Vec<(String, Result<(), String>)> {
        std::mem::take(&mut self.lock().settled)
    }

    /// Whether message `id` waits for an acknowledgement.
    pub fn is_pending(&self, id: &str) -> bool {
        self.lock().pending.contains_key(id)
    }

    /// The number of messages waiting for an acknowledgement.
    pub fn pending_len(&self) -> usize {
        self.lock().pending.len()
    }

    /// A connection with number `generation` is open (and accepted): its writer takes the
    /// frame texts sent into `tx`. Every pending message is sent again, oldest first, and a
    /// refusal is forgotten.
    pub fn connected(&self, generation: u64, tx: UnboundedSender<String>) {
        let mut s = self.lock();
        s.connection = Some((generation, tx));
        s.refused = None;
        let mut ids: Vec<(u64, String)> = s
            .pending
            .iter()
            .map(|(k, p)| (p.order, k.clone()))
            .collect();
        ids.sort();
        for (_, id) in ids {
            Link::send_pending(&mut s, &id);
        }
    }

    /// The connection `generation` closed (a later connection is left alone).
    pub fn disconnected(&self, generation: u64) {
        let mut s = self.lock();
        if s.connection.as_ref().is_some_and(|c| c.0 == generation) {
            s.connection = None;
        }
    }

    /// Whether a connection is open.
    pub fn is_connected(&self) -> bool {
        self.lock().connection.is_some()
    }

    /// Sends an acknowledgement of message `id` on the current connection.
    pub fn ack(&self, id: &str) {
        let s = self.lock();
        if let Some((_, tx)) = &s.connection {
            let _ = tx.unbounded_send(Frame::Ack { id: id.to_owned() }.to_text());
        }
    }

    /// Sends a frame on the current connection; false when there is none.
    pub fn send_frame(&self, f: &Frame) -> bool {
        let s = self.lock();
        match &s.connection {
            Some((_, tx)) => tx.unbounded_send(f.to_text()).is_ok(),
            None => false,
        }
    }

    /// The credential was refused: watched pending messages fail, the others are dropped from
    /// the link (the outbox offers them again and learns of the refusal then). The refusal
    /// holds until the next accepted connection.
    pub fn refused(&self, reason: &str) -> Vec<String> {
        let mut s = self.lock();
        s.refused = Some(reason.to_owned());
        let ids: Vec<String> = s.pending.keys().cloned().collect();
        s.pending.clear();
        let mut failed = Vec::new();
        for id in ids {
            if s.watched.remove(&id) {
                s.settled.push((id.clone(), Err(reason.to_owned())));
                failed.push(id);
            }
        }
        failed
    }

    /// Whether the peer refused the credential (and no connection was accepted since).
    pub fn refusal(&self) -> Option<String> {
        self.lock().refused.clone()
    }

    /// Learns that actor `iri` is reachable through this link (the origin of a message it
    /// sent).
    pub fn learn_route(&self, iri: &str) {
        self.lock().routes.insert(iri.to_owned());
    }

    /// Whether actor `iri` is known to be reachable through this link.
    pub fn routes_to(&self, iri: &str) -> bool {
        self.lock().routes.contains(iri)
    }

    /// Handles one received frame text.
    ///
    /// # Errors
    ///
    /// A message for text that is not a frame.
    pub fn receive(&self, text: &str) -> Result<Inbound, String> {
        Ok(match Frame::parse(text)? {
            Frame::Message(m) => {
                if let Some(o) = &m.origin {
                    self.learn_route(o);
                }
                Inbound::Deliver(m.into_envelope())
            }
            Frame::Ack { id } => {
                let mut s = self.lock();
                let known = s.pending.remove(&id).is_some();
                let watched = s.watched.remove(&id);
                if watched {
                    s.settled.push((id.clone(), Ok(())));
                } else if known {
                    s.acked.insert(id.clone());
                }
                Inbound::Acked { id, watched }
            }
            Frame::Refused { reason } => {
                let failed = self.refused(&reason);
                Inbound::Refused { reason, failed }
            }
            f @ (Frame::Hello { .. } | Frame::Welcome { .. }) => Inbound::Handshake(f),
        })
    }
}

/// The runtime's view of an actor reached through a [`Link`]: a mailbox that offers each
/// message to the link. It accepts a message once the peer acknowledged it; until then it
/// reports the delivery as failed, so the outbox keeps the send and retries it. A send refused
/// by the peer's credential check is recorded as a dead letter (reason `transport-refused`)
/// and settled.
pub struct LinkMailbox {
    link: Arc<Link>,
    handle: Option<NodeHandle>,
}

impl std::fmt::Debug for LinkMailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkMailbox")
            .field("link", &self.link)
            .finish_non_exhaustive()
    }
}

impl LinkMailbox {
    /// A mailbox over `link`; `handle` records dead letters of refused sends.
    pub fn new(link: Arc<Link>, handle: Option<NodeHandle>) -> Self {
        LinkMailbox { link, handle }
    }
}

impl Mailbox for LinkMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        let (to, copy) = (env.to.clone(), env.clone());
        match self.link.offer(env) {
            Offer::Delivered => Ok(()),
            Offer::Pending => Err(MailboxError(
                "awaiting the WebSocket peer's acknowledgement".into(),
            )),
            Offer::Refused(_) => {
                if let (Some(h), Ok(a)) = (&self.handle, Address::parse(&to)) {
                    h.record_dead_letter(&a, &copy, REFUSED_REASON);
                }
                Ok(())
            }
        }
    }

    fn receive(&self, _deferred: &[String]) -> Option<Envelope> {
        None
    }

    fn ack(&self, _message_id: &str) -> Result<(), MailboxError> {
        Ok(())
    }

    fn release(&self, _message_id: &str) {}

    fn release_all(&self) {}

    fn pending(&self) -> Vec<Envelope> {
        Vec::new()
    }

    fn pending_deliverable(&self, _deferred: &[String]) -> usize {
        0
    }
}

/// Resolves the addresses a client sends to over its one [`Link`] to the server: every
/// address (of the schemes it is registered for) that starts with one of `targets`, or every
/// address when `targets` is empty. Addresses hosted locally never reach a resolver.
pub struct ClientResolver {
    link: Arc<Link>,
    handle: Option<NodeHandle>,
    targets: Vec<String>,
}

impl std::fmt::Debug for ClientResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientResolver")
            .field("targets", &self.targets)
            .finish_non_exhaustive()
    }
}

impl ClientResolver {
    /// A resolver sending the addresses under `targets` over `link`.
    pub fn new(link: Arc<Link>, handle: Option<NodeHandle>, targets: Vec<String>) -> Self {
        ClientResolver {
            link,
            handle,
            targets,
        }
    }

    /// Whether `address` goes over the link.
    pub fn claims(&self, address: &str) -> bool {
        self.targets.is_empty() || self.targets.iter().any(|t| address.starts_with(t.as_str()))
    }
}

impl Resolver for ClientResolver {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        self.claims(address.as_str()).then(|| {
            Arc::new(LinkMailbox::new(self.link.clone(), self.handle.clone())) as Arc<dyn Mailbox>
        })
    }
}

/// The WebSocket Event I/O Processor as a runtime transport (`<send type="websocket">` or its
/// type IRI). It delivers into the mailbox the address resolved to, which for a remote actor
/// is a [`LinkMailbox`]; `_ioprocessors` lists it.
#[derive(Clone, Copy, Debug, Default)]
pub struct WebSocketTransport;

impl Transport for WebSocketTransport {
    fn type_iri(&self) -> &str {
        WEBSOCKET_PROCESSOR
    }

    fn aliases(&self) -> Vec<String> {
        vec!["websocket".to_owned()]
    }

    fn deliver(&self, mailbox: &dyn Mailbox, env: Envelope) -> Result<(), TransportError> {
        mailbox
            .enqueue(env)
            .map_err(|e| TransportError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc::unbounded;
    use vertep_core::datamodel::Value;

    fn env(id: &str) -> Envelope {
        let mut e = Event::named("ping", EventType::External);
        e.origin = Some("https://client.example/a/1".into());
        e.data = Value::Object(vec![("n".into(), Value::Number(1.0))]);
        Envelope::new(
            id,
            &Address::parse("https://server.example/s/1").unwrap(),
            e,
        )
    }

    fn drain(rx: &mut futures::channel::mpsc::UnboundedReceiver<String>) -> Vec<Frame> {
        let mut out = Vec::new();
        while let Ok(t) = rx.try_recv() {
            out.push(Frame::parse(&t).unwrap());
        }
        out
    }

    #[test]
    fn frames_round_trip() {
        let m = Message::from_envelope(&env("m1"));
        let f = Frame::Message(m.clone());
        assert_eq!(Frame::parse(&f.to_text()).unwrap(), f);
        let delivered = m.into_envelope();
        assert_eq!(
            delivered.event.origintype.as_deref(),
            Some(WEBSOCKET_PROCESSOR)
        );
        assert_eq!(
            delivered.event.origin.as_deref(),
            Some("https://client.example/a/1")
        );
        assert_eq!(delivered.event.data, env("m1").event.data);
        assert!(Frame::parse(r#"{"type":"ack","id":"x"}"#).is_ok());
        assert!(Frame::parse("nope").is_err());
    }

    #[test]
    fn pending_until_acked_and_resent_after_reconnect() {
        let link = Link::new();
        assert_eq!(link.offer(env("m1")), Offer::Pending, "no connection yet");
        let (tx, mut rx) = unbounded();
        link.connected(1, tx);
        assert_eq!(drain(&mut rx).len(), 1, "sent on connect");
        assert_eq!(link.offer(env("m1")), Offer::Pending);
        assert!(
            drain(&mut rx).is_empty(),
            "a retry does not resend on the same connection"
        );
        link.disconnected(1);
        let (tx, mut rx) = unbounded();
        link.connected(2, tx);
        assert_eq!(drain(&mut rx).len(), 1, "resent after reconnect");
        link.receive(r#"{"type":"ack","id":"m1"}"#).unwrap();
        assert_eq!(link.offer(env("m1")), Offer::Delivered);
        assert_eq!(link.pending_len(), 0);
    }

    #[test]
    fn watched_messages_settle_and_fail_on_refusal() {
        let link = Link::new();
        let (tx, _rx) = unbounded();
        link.connected(1, tx);
        link.offer(env("a"));
        link.watch("a");
        link.offer(env("b"));
        link.watch("b");
        link.receive(r#"{"type":"ack","id":"a"}"#).unwrap();
        assert_eq!(link.take_settled(), vec![("a".into(), Ok(()))]);
        let r = link
            .receive(r#"{"type":"refused","reason":"bad token"}"#)
            .unwrap();
        assert_eq!(
            r,
            Inbound::Refused {
                reason: "bad token".into(),
                failed: vec!["b".into()]
            }
        );
        assert_eq!(
            link.take_settled(),
            vec![("b".into(), Err("bad token".into()))]
        );
        assert_eq!(link.offer(env("c")), Offer::Refused("bad token".into()));
        let (tx, mut rx) = unbounded();
        link.connected(2, tx);
        assert_eq!(link.offer(env("c")), Offer::Pending, "accepted again");
        assert!(rx.try_recv().is_ok(), "and sent");
    }

    #[test]
    fn received_messages_teach_routes() {
        let link = Link::new();
        let text = Frame::Message(Message::from_envelope(&env("m"))).to_text();
        assert!(matches!(link.receive(&text).unwrap(), Inbound::Deliver(_)));
        assert!(link.routes_to("https://client.example/a/1"));
    }
}
