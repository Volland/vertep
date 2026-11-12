//! The TSP mailbox protocol (feature `tsp`): registration, status, delivery, acknowledgement and
//! live sessions between a VID and its TSP mediator, carried as TSP application messages over
//! their relationship, and the client side of it as a [`PickupSource`].
//!
//! TSP 1.0 routes messages through intermediaries but defines no store-and-forward, so a
//! recipient that is offline needs a mailbox protocol on top (design D10). Each message is a
//! JSON object `{id, type, thid?, body}` sealed in a TSP generic payload; types are IRIs under
//! [`MAILBOX_NS`], versioned so the protocol can evolve:
//!
//! | type | direction | body |
//! |---|---|---|
//! | `register` | VID → mediator | `{}` |
//! | `registered` | mediator → VID | `{recipient, mediator, endpoint, routing}` |
//! | `unregister` | VID → mediator | `{}` |
//! | `unregistered` | mediator → VID | `{recipient}` |
//! | `status` | both | request `{}`; reply `{message_count, total_bytes, oldest_age_ms, expired}` |
//! | `deliver` | VID → mediator | `{limit}` |
//! | `batch` | mediator → VID | `{messages: [{id, size, arrival_ms, data}], remaining, expired}` |
//! | `received` | VID → mediator | `{ids}`; answered with `status` |
//! | `live` | VID → mediator | `{}`, the first frame of a live session |
//! | `problem` | mediator → VID | `{code, comment}` |
//!
//! A message stays at the mediator until a `received` names it; a `batch` is not an
//! acknowledgement. [`TspMailboxClient`] acknowledges only when the runtime acknowledges after
//! the journal append, as every relay mailbox does.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tsp_sdk::cesr::EnvelopeType;
use tsp_sdk::definitions::Payload;
use tsp_sdk::{VerifiedVid, Vid};

use crate::did::{Did, Service, ServiceEndpoint};
use crate::live::{FrameSink, LiveNetwork, live_uri};
use crate::net::{NetError, PickupSource, RelayNetwork, StoredMessage};
use crate::resolver::DidResolver;
use crate::tsp::{Relationship, TspError, TspIdentity, TspReceived, TspSession, vid_from_document};

/// The namespace of the mailbox protocol's message types.
pub const MAILBOX_NS: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#";
/// `register`: ask for a mailbox.
pub const REGISTER: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#register";
/// `registered`: the mailbox exists.
pub const REGISTERED: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#registered";
/// `unregister`: give the mailbox up.
pub const UNREGISTER: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#unregister";
/// `unregistered`: the mailbox is gone.
pub const UNREGISTERED: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#unregistered";
/// `status`: request or report of what is waiting.
pub const STATUS: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#status";
/// `deliver`: ask for a batch.
pub const DELIVER: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#deliver";
/// `batch`: waiting messages.
pub const BATCH: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#batch";
/// `received`: acknowledge messages, which the mediator may then delete.
pub const RECEIVED: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#received";
/// `live`: open a live session.
pub const LIVE: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#live";
/// `problem`: a refused request.
pub const PROBLEM: &str = "https://w3id.org/vertep/tsp/mailbox/1.0#problem";

/// One mailbox protocol message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MailboxMessage {
    /// The message id.
    pub id: String,
    /// The message type, an IRI under [`MAILBOX_NS`].
    #[serde(rename = "type")]
    pub type_: String,
    /// The id of the request this message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thid: Option<String>,
    /// The body.
    #[serde(default)]
    pub body: Value,
}

impl MailboxMessage {
    /// A request of `type_` with a fresh id.
    pub fn request(type_: &str, body: Value) -> Self {
        MailboxMessage {
            id: new_id(),
            type_: type_.to_owned(),
            thid: None,
            body,
        }
    }

    /// The reply of `type_` to `request`.
    pub fn reply(request: &MailboxMessage, type_: &str, body: Value) -> Self {
        MailboxMessage {
            id: new_id(),
            type_: type_.to_owned(),
            thid: Some(request.id.clone()),
            body,
        }
    }

    /// A `problem` reply to `request` (or unsolicited when `None`).
    pub fn problem(request: Option<&MailboxMessage>, code: &str, comment: &str) -> Self {
        MailboxMessage {
            id: new_id(),
            type_: PROBLEM.to_owned(),
            thid: request.map(|r| r.id.clone()),
            body: json!({"code": code, "comment": comment}),
        }
    }
}

/// A message held at the mediator, as a `batch` carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldMessage {
    /// The mediator's id of the stored copy, used in `received`.
    pub id: String,
    /// The size of the sealed message in bytes.
    pub size: u64,
    /// When the mediator stored it (milliseconds since the epoch).
    pub arrival_ms: i64,
    /// The sealed message, opaque to the mediator.
    pub data: Vec<u8>,
}

impl HeldMessage {
    /// The JSON form in a `batch`.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "size": self.size,
            "arrival_ms": self.arrival_ms,
            "data": STANDARD.encode(&self.data),
        })
    }

    /// Reads the JSON form.
    ///
    /// # Errors
    ///
    /// A description when a member is missing or the data is not base64.
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or("batch message without id")?
            .to_owned();
        let data = v
            .get("data")
            .and_then(Value::as_str)
            .ok_or("batch message without data")?;
        let data = STANDARD
            .decode(data)
            .map_err(|e| format!("batch message {id}: {e}"))?;
        Ok(HeldMessage {
            size: v
                .get("size")
                .and_then(Value::as_u64)
                .unwrap_or(data.len() as u64),
            arrival_ms: v.get("arrival_ms").and_then(Value::as_i64).unwrap_or(0),
            id,
            data,
        })
    }
}

/// Messages that expired at the mediator before pickup since the last report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    /// How many expired.
    pub count: u64,
    /// When the oldest of them expired (milliseconds since the epoch), 0 when none did.
    pub oldest_expiry_ms: i64,
}

impl ExpiryReport {
    /// The JSON form (`{count, oldest_expiry_ms}`).
    pub fn to_json(self) -> Value {
        json!({"count": self.count, "oldest_expiry_ms": self.oldest_expiry_ms})
    }

    /// Reads the JSON form; absent members are zero.
    pub fn from_json(v: Option<&Value>) -> Self {
        let v = v.cloned().unwrap_or(Value::Null);
        ExpiryReport {
            count: v.get("count").and_then(Value::as_u64).unwrap_or(0),
            oldest_expiry_ms: v
                .get("oldest_expiry_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        }
    }
}

/// A mailbox's status at the mediator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailboxStatus {
    /// Waiting messages.
    pub message_count: u64,
    /// Their total size in bytes.
    pub total_bytes: u64,
    /// The age of the oldest, in milliseconds (0 when none waits).
    pub oldest_age_ms: i64,
    /// Expiries since the last report.
    pub expired: ExpiryReport,
}

impl MailboxStatus {
    /// The JSON body of a `status` reply.
    pub fn to_json(self) -> Value {
        json!({
            "message_count": self.message_count,
            "total_bytes": self.total_bytes,
            "oldest_age_ms": self.oldest_age_ms,
            "expired": self.expired.to_json(),
        })
    }

    /// Reads a `status` body.
    pub fn from_json(v: &Value) -> Self {
        MailboxStatus {
            message_count: v.get("message_count").and_then(Value::as_u64).unwrap_or(0),
            total_bytes: v.get("total_bytes").and_then(Value::as_u64).unwrap_or(0),
            oldest_age_ms: v.get("oldest_age_ms").and_then(Value::as_i64).unwrap_or(0),
            expired: ExpiryReport::from_json(v.get("expired")),
        }
    }
}

/// A `batch`: messages and what remains.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    /// The delivered messages, oldest first.
    pub messages: Vec<HeldMessage>,
    /// Messages still waiting after these.
    pub remaining: u64,
    /// Expiries since the last report.
    pub expired: ExpiryReport,
}

impl Batch {
    /// The JSON body of a `batch`.
    pub fn to_json(&self) -> Value {
        json!({
            "messages": self.messages.iter().map(HeldMessage::to_json).collect::<Vec<_>>(),
            "remaining": self.remaining,
            "expired": self.expired.to_json(),
        })
    }

    /// Reads a `batch` body.
    ///
    /// # Errors
    ///
    /// A description of the first malformed message.
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let messages = v
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(HeldMessage::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Batch {
            messages,
            remaining: v.get("remaining").and_then(Value::as_u64).unwrap_or(0),
            expired: ExpiryReport::from_json(v.get("expired")),
        })
    }
}

/// What the mediator answered to `register`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    /// The registered recipient VID.
    pub recipient: String,
    /// The mediator's VID.
    pub mediator: String,
    /// The mediator's TSP endpoint.
    pub endpoint: String,
    /// The VIDs senders route through to reach the recipient (the mediator last).
    pub routing: Vec<String>,
}

impl Registration {
    /// The `TSPTransport` service the recipient publishes so senders route through the
    /// mediator: its own endpoint `own_uri` (which may be unreachable) with the mediator as the
    /// last intermediary.
    pub fn service(&self, own_uri: &str) -> Service {
        Service {
            id: "#tsp".into(),
            types: vec!["TSPTransport".into()],
            endpoints: vec![ServiceEndpoint {
                uri: own_uri.to_owned(),
                routing_keys: self.routing.clone(),
                accept: Vec::new(),
            }],
        }
    }
}

/// Why a mailbox exchange failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailboxError {
    /// The mediator could not be reached.
    Net(NetError),
    /// Sealing or opening failed.
    Tsp(String),
    /// The mediator has no usable TSP endpoint or keys.
    NoEndpoint(String),
    /// The mediator refused the relationship.
    RelationshipRefused(String),
    /// The mediator answered with a `problem`.
    Problem {
        /// The problem code (`not-registered`, `not-owner`, ...).
        code: String,
        /// The comment.
        comment: String,
    },
    /// The reply was not from the mediator, did not answer the request, or was malformed.
    Unexpected(String),
}

impl fmt::Display for MailboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MailboxError::Net(e) => write!(f, "mediator: {e}"),
            MailboxError::Tsp(m) => write!(f, "mediator TSP: {m}"),
            MailboxError::NoEndpoint(m) => write!(f, "mediator has no TSP endpoint: {m}"),
            MailboxError::RelationshipRefused(m) => write!(f, "mediator refused relationship: {m}"),
            MailboxError::Problem { code, comment } => {
                write!(f, "mediator problem {code}: {comment}")
            }
            MailboxError::Unexpected(m) => write!(f, "unexpected mediator reply: {m}"),
        }
    }
}

impl std::error::Error for MailboxError {}

impl From<NetError> for MailboxError {
    fn from(e: NetError) -> Self {
        MailboxError::Net(e)
    }
}

impl From<TspError> for MailboxError {
    fn from(e: TspError) -> Self {
        match e {
            TspError::RelationshipRefused(m) => MailboxError::RelationshipRefused(m),
            TspError::NoVid(m) => MailboxError::NoEndpoint(m),
            other => MailboxError::Tsp(other.to_string()),
        }
    }
}

impl From<MailboxError> for NetError {
    fn from(e: MailboxError) -> Self {
        match e {
            MailboxError::Net(n) => n,
            other => NetError::Refused(other.to_string()),
        }
    }
}

/// A fresh random id (128 bits, hex).
pub fn new_id() -> String {
    let mut b = [0u8; 16];
    // A failing system RNG leaves zeros; the id is a label, not a secret.
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The sender and receiver VIDs named in a TSP message's envelope, if it is an encrypted
/// message.
pub fn envelope_ids(bytes: &[u8]) -> Option<(String, String)> {
    let mut copy = bytes.to_vec();
    match tsp_sdk::cesr::probe(&mut copy).ok()? {
        EnvelopeType::EncryptedMessage { sender, receiver } => Some((
            String::from_utf8_lossy(sender).into_owned(),
            String::from_utf8_lossy(receiver).into_owned(),
        )),
        EnvelopeType::SignedMessage { .. } => None,
    }
}

/// Seals `message` from `me` to `peer` as a TSP generic (content) payload.
///
/// # Errors
///
/// [`TspError::Seal`] when sealing fails.
pub fn seal_message(
    me: &TspIdentity,
    peer: &Vid,
    message: &MailboxMessage,
) -> Result<Vec<u8>, TspError> {
    let body = serde_json::to_vec(message).map_err(|e| TspError::Seal(e.to_string()))?;
    tsp_sdk::crypto::seal(me, peer, Payload::Content(&body))
        .map_err(|e| TspError::Seal(e.to_string()))
}

/// What a TSP message for a mailbox party carried.
#[derive(Clone, Debug, PartialEq)]
pub enum Opened {
    /// A mailbox protocol message from the authenticated `sender`.
    Mailbox {
        /// The sender VID, authenticated by its signature.
        sender: String,
        /// The message.
        message: MailboxMessage,
    },
    /// Anything else (a relationship control message, a routed or nested message), left to
    /// the TSP session or the routing code.
    Other(PayloadKind),
}

/// The kind of a TSP payload that is not a mailbox message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    /// A relationship request, accept or cancel.
    Relationship,
    /// A routed message for an intermediary.
    Routed,
    /// A nested message for the final recipient.
    Nested,
    /// Content that is not a mailbox message (an event envelope).
    Content,
    /// Something else.
    Unsupported,
}

/// Opens a TSP message addressed to `me` without consuming its routing or relationship
/// semantics: a mailbox message is returned parsed with its authenticated sender, anything else
/// only by kind so the caller can hand the untouched bytes to [`TspSession::open`] or
/// [`crate::tsp::forward_routed`].
///
/// # Errors
///
/// [`TspError::Open`] when the message is not for `me`, does not open, or its sender does not
/// resolve.
pub fn open_message(
    me: &TspIdentity,
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Result<Vid, TspError>,
) -> Result<Opened, TspError> {
    let (sender, receiver) =
        envelope_ids(bytes).ok_or_else(|| TspError::Open("not an encrypted TSP message".into()))?;
    if receiver != me.identifier() {
        return Err(TspError::Open(format!("addressed to {receiver}")));
    }
    let sender_vid = resolve(&sender)?;
    let mut copy = bytes.to_vec();
    let (payload, _, _) = tsp_sdk::crypto::open(me, &sender_vid, &mut copy)
        .map_err(|e| TspError::Open(e.to_string()))?;
    Ok(match payload {
        Payload::Content(body) => match serde_json::from_slice::<MailboxMessage>(body) {
            Ok(m) if m.type_.starts_with(MAILBOX_NS) => Opened::Mailbox { sender, message: m },
            _ => Opened::Other(PayloadKind::Content),
        },
        Payload::RequestRelationship { .. }
        | Payload::AcceptRelationship { .. }
        | Payload::CancelRelationship { .. } => Opened::Other(PayloadKind::Relationship),
        Payload::RoutedMessage(..) => Opened::Other(PayloadKind::Routed),
        Payload::NestedMessage(..) => Opened::Other(PayloadKind::Nested),
        _ => Opened::Other(PayloadKind::Unsupported),
    })
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn resolve_vid(resolver: &dyn DidResolver, id: &str) -> Result<Vid, TspError> {
    let did = Did::parse(id).map_err(|e| TspError::NoVid(e.to_string()))?;
    let doc = resolver
        .resolve(&did)
        .map_err(|e| TspError::NoVid(e.to_string()))?
        .document;
    vid_from_document(&doc)
}

type Waker = Arc<dyn Fn(&str) + Send + Sync>;

struct LiveState {
    network: Arc<dyn LiveNetwork>,
    uri: String,
    /// The open session per recipient VID: the sink of frames to the mediator.
    sessions: Mutex<HashMap<String, Arc<dyn FrameSink>>>,
}

#[derive(Default)]
struct ClientState {
    /// Registered actors' sessions, by VID.
    actors: BTreeMap<String, Arc<TspSession>>,
    /// Which recipient each fetched copy belongs to, for `received`.
    issued: HashMap<String, String>,
    /// Messages pushed on live sessions, not yet fetched by the inbox.
    pushed: VecDeque<StoredMessage>,
    /// Expiry reports received, by recipient.
    expired: BTreeMap<String, ExpiryReport>,
}

/// The client of a TSP mediator for the local actors that registered with it: a relationship
/// per actor VID, the mailbox protocol over it, and a [`PickupSource`] for the relay inbox, so a
/// TSP actor's messages wait at the mediator while it is deactivated or offline.
// @tg: implements:: [[openspec:relay-mailboxes#TSP Mediator Mailbox]]
// @tg: implements:: [[openspec:relay-mailboxes#Live Delivery Over WebSocket]] {note: "TSP live sessions"}
pub struct TspMailboxClient {
    mediator: Vid,
    endpoint: String,
    net: Arc<dyn RelayNetwork>,
    resolver: Arc<dyn DidResolver>,
    state: Mutex<ClientState>,
    live: Option<Arc<LiveState>>,
    me: std::sync::Weak<TspMailboxClient>,
}

impl fmt::Debug for TspMailboxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspMailboxClient")
            .field("mediator", &self.mediator.identifier())
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl TspMailboxClient {
    /// A client of the mediator `mediator_did`, resolved with `resolver`; requests are posted to
    /// its `TSPTransport` endpoint through `net`.
    ///
    /// # Errors
    ///
    /// [`MailboxError::NoEndpoint`] when the mediator does not resolve or lacks keys or an
    /// endpoint.
    pub fn new(
        mediator_did: &str,
        net: Arc<dyn RelayNetwork>,
        resolver: Arc<dyn DidResolver>,
    ) -> Result<Arc<Self>, MailboxError> {
        Self::build(mediator_did, net, resolver, None)
    }

    /// Like [`TspMailboxClient::new`], with live sessions over `live` at the mediator's live
    /// endpoint ([`live_uri`] of its TSP endpoint).
    ///
    /// # Errors
    ///
    /// As [`TspMailboxClient::new`].
    pub fn with_live(
        mediator_did: &str,
        net: Arc<dyn RelayNetwork>,
        resolver: Arc<dyn DidResolver>,
        live: Arc<dyn LiveNetwork>,
    ) -> Result<Arc<Self>, MailboxError> {
        Self::build(mediator_did, net, resolver, Some(live))
    }

    fn build(
        mediator_did: &str,
        net: Arc<dyn RelayNetwork>,
        resolver: Arc<dyn DidResolver>,
        live: Option<Arc<dyn LiveNetwork>>,
    ) -> Result<Arc<Self>, MailboxError> {
        let did = Did::parse(mediator_did)
            .map_err(|e| MailboxError::NoEndpoint(format!("{mediator_did}: {e}")))?;
        let doc = resolver
            .resolve(&did)
            .map_err(|e| MailboxError::NoEndpoint(format!("{mediator_did}: {e}")))?
            .document;
        let endpoint = crate::tsp::tsp_uri(&doc)
            .ok_or_else(|| MailboxError::NoEndpoint(mediator_did.to_owned()))?;
        let mediator = vid_from_document(&doc)?;
        let live = live.map(|network| {
            Arc::new(LiveState {
                network,
                uri: live_uri(&endpoint),
                sessions: Mutex::new(HashMap::new()),
            })
        });
        Ok(Arc::new_cyclic(|me| TspMailboxClient {
            mediator,
            endpoint,
            net,
            resolver,
            state: Mutex::new(ClientState::default()),
            live,
            me: me.clone(),
        }))
    }

    /// The mediator's VID.
    pub fn mediator_did(&self) -> &str {
        self.mediator.identifier()
    }

    /// The mediator's endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn resolve(&self, id: &str) -> Result<Vid, TspError> {
        if id == self.mediator.identifier() {
            return Ok(self.mediator.clone());
        }
        resolve_vid(self.resolver.as_ref(), id)
    }

    /// Sets up the TSP relationship of `actor` with the mediator, if not already established.
    ///
    /// # Errors
    ///
    /// [`MailboxError::RelationshipRefused`] when the mediator refuses; network and sealing
    /// failures.
    pub fn connect(&self, actor: &TspSession) -> Result<(), MailboxError> {
        let mediator = self.mediator.identifier();
        if actor.relationship(mediator) == Some(Relationship::Established) {
            return Ok(());
        }
        let rfi = actor.request_relationship(&self.mediator)?;
        let mut reply = self.net.post(&self.endpoint, &rfi.bytes)?;
        if reply.is_empty() {
            return Err(MailboxError::RelationshipRefused(mediator.to_owned()));
        }
        match actor.open(&mut reply, &|id| self.resolve(id))? {
            TspReceived::RelationshipAccepted { .. } => Ok(()),
            TspReceived::RelationshipCancelled { sender } => {
                Err(MailboxError::RelationshipRefused(sender))
            }
            other => Err(MailboxError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Sends one request from `actor` and returns the mediator's reply, authenticated as the
    /// mediator's and threaded to the request. A `problem` becomes [`MailboxError::Problem`].
    ///
    /// # Errors
    ///
    /// [`MailboxError`] for network, sealing and opening failures and unexpected replies.
    pub fn exchange(
        &self,
        actor: &TspSession,
        type_: &str,
        body: Value,
    ) -> Result<MailboxMessage, MailboxError> {
        self.connect(actor)?;
        let request = MailboxMessage::request(type_, body);
        let sealed = seal_message(actor.identity(), &self.mediator, &request)?;
        let reply = self.net.post(&self.endpoint, &sealed)?;
        if reply.is_empty() {
            return Err(MailboxError::Unexpected(format!("no reply to {type_}")));
        }
        let m = self.open_reply(actor, &reply)?;
        if m.type_ == PROBLEM {
            return Err(problem(&m));
        }
        if m.thid.as_deref() != Some(request.id.as_str()) {
            return Err(MailboxError::Unexpected(format!(
                "{} does not answer {type_} {}",
                m.type_, request.id
            )));
        }
        Ok(m)
    }

    fn open_reply(&self, actor: &TspSession, bytes: &[u8]) -> Result<MailboxMessage, MailboxError> {
        match open_message(actor.identity(), bytes, &|id| self.resolve(id))? {
            Opened::Mailbox { sender, message } if sender == self.mediator.identifier() => {
                Ok(message)
            }
            Opened::Mailbox { sender, .. } => Err(MailboxError::Unexpected(format!(
                "reply from {sender}, not the mediator"
            ))),
            Opened::Other(k) => Err(MailboxError::Unexpected(format!("reply is {k:?}"))),
        }
    }

    /// Registers `actor` for a mailbox at the mediator and remembers it, so the client picks up
    /// for it.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] when the relationship or registration is refused.
    pub fn register(&self, actor: Arc<TspSession>) -> Result<Registration, MailboxError> {
        let m = self.exchange(&actor, REGISTER, json!({}))?;
        expect(&m, REGISTERED)?;
        let b = &m.body;
        let reg = Registration {
            recipient: str_of(b, "recipient"),
            mediator: str_of(b, "mediator"),
            endpoint: str_of(b, "endpoint"),
            routing: b
                .get("routing")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
        };
        if reg.recipient != actor.identity().identifier() {
            return Err(MailboxError::Unexpected(format!(
                "registered {} for {}",
                reg.recipient,
                actor.identity().identifier()
            )));
        }
        lock(&self.state)
            .actors
            .insert(reg.recipient.clone(), actor);
        Ok(reg)
    }

    /// Gives up `vid`'s mailbox.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] for failed exchanges or an unknown actor.
    pub fn unregister(&self, vid: &str) -> Result<(), MailboxError> {
        let actor = self.actor(vid)?;
        let m = self.exchange(&actor, UNREGISTER, json!({}))?;
        expect(&m, UNREGISTERED)?;
        let mut s = lock(&self.state);
        s.actors.remove(vid);
        s.pushed.retain(|p| p.recipient != vid);
        Ok(())
    }

    fn actor(&self, vid: &str) -> Result<Arc<TspSession>, MailboxError> {
        lock(&self.state)
            .actors
            .get(vid)
            .cloned()
            .ok_or_else(|| MailboxError::Unexpected(format!("{vid} is not registered here")))
    }

    /// The status of `vid`'s mailbox.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] for failed exchanges.
    pub fn status(&self, vid: &str) -> Result<MailboxStatus, MailboxError> {
        let actor = self.actor(vid)?;
        let m = self.exchange(&actor, STATUS, json!({}))?;
        expect(&m, STATUS)?;
        let st = MailboxStatus::from_json(&m.body);
        self.note_expired(vid, st.expired);
        Ok(st)
    }

    /// Up to `limit` waiting messages of `vid`; they stay at the mediator until
    /// [`TspMailboxClient::received`].
    ///
    /// # Errors
    ///
    /// [`MailboxError`] for failed exchanges.
    pub fn deliver(&self, vid: &str, limit: usize) -> Result<Batch, MailboxError> {
        let actor = self.actor(vid)?;
        let m = self.exchange(&actor, DELIVER, json!({ "limit": limit }))?;
        expect(&m, BATCH)?;
        let batch = Batch::from_json(&m.body).map_err(MailboxError::Unexpected)?;
        self.note_expired(vid, batch.expired);
        Ok(batch)
    }

    /// Asks for `owner`'s messages while authenticated as `vid`: what a VID other than the
    /// recipient would send. The mediator must refuse.
    ///
    /// # Errors
    ///
    /// [`MailboxError::Problem`] when refused (`not-owner`).
    pub fn deliver_for(&self, vid: &str, owner: &str, limit: usize) -> Result<Batch, MailboxError> {
        let actor = self.actor(vid)?;
        let m = self.exchange(
            &actor,
            DELIVER,
            json!({ "limit": limit, "recipient": owner }),
        )?;
        expect(&m, BATCH)?;
        Batch::from_json(&m.body).map_err(MailboxError::Unexpected)
    }

    /// Acknowledges `ids` of `vid`'s mailbox; the mediator deletes them.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] for failed exchanges.
    pub fn received(&self, vid: &str, ids: &[String]) -> Result<MailboxStatus, MailboxError> {
        let actor = self.actor(vid)?;
        let m = self.exchange(&actor, RECEIVED, json!({ "ids": ids }))?;
        expect(&m, STATUS)?;
        Ok(MailboxStatus::from_json(&m.body))
    }

    /// The expiries reported for `vid` so far (accumulated).
    pub fn expired(&self, vid: &str) -> ExpiryReport {
        lock(&self.state)
            .expired
            .get(vid)
            .copied()
            .unwrap_or_default()
    }

    fn note_expired(&self, vid: &str, e: ExpiryReport) {
        if e.count == 0 {
            return;
        }
        let mut s = lock(&self.state);
        let r = s.expired.entry(vid.to_owned()).or_default();
        if r.count == 0 || (e.oldest_expiry_ms != 0 && e.oldest_expiry_ms < r.oldest_expiry_ms) {
            r.oldest_expiry_ms = e.oldest_expiry_ms;
        }
        r.count += e.count;
    }

    /// True when `vid` holds an open live session.
    pub fn is_live(&self, vid: &str) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| lock(&l.sessions).contains_key(vid))
    }

    fn open_live(&self, vid: &str, wake: Waker) -> Result<(), MailboxError> {
        let Some(live) = &self.live else {
            return Err(MailboxError::Unexpected("no live network".into()));
        };
        let actor = self.actor(vid)?;
        self.connect(&actor)?;
        let sink = Arc::new(LiveFrames {
            client: self.me.clone(),
            vid: vid.to_owned(),
            wake,
        });
        let up = live.network.connect(&live.uri, sink)?;
        let hello = MailboxMessage::request(LIVE, json!({}));
        up.send(&seal_message(actor.identity(), &self.mediator, &hello)?)?;
        lock(&live.sessions).insert(vid.to_owned(), up);
        Ok(())
    }

    fn on_frame(&self, vid: &str, frame: &[u8], wake: &Waker) {
        let Ok(actor) = self.actor(vid) else {
            return;
        };
        let Ok(m) = self.open_reply(&actor, frame) else {
            return;
        };
        if m.type_ != BATCH {
            return;
        }
        let Ok(batch) = Batch::from_json(&m.body) else {
            return;
        };
        self.note_expired(vid, batch.expired);
        let mut fresh = false;
        {
            let mut s = lock(&self.state);
            for h in batch.messages {
                if s.pushed.iter().any(|p| p.id == h.id) {
                    continue;
                }
                s.issued.insert(h.id.clone(), vid.to_owned());
                s.pushed.push_back(StoredMessage {
                    id: h.id,
                    recipient: vid.to_owned(),
                    bytes: h.data,
                });
                fresh = true;
            }
        }
        if fresh {
            wake(vid);
        }
    }

    fn on_closed(&self, vid: &str) {
        if let Some(live) = &self.live {
            lock(&live.sessions).remove(vid);
        }
    }
}

fn expect(m: &MailboxMessage, type_: &str) -> Result<(), MailboxError> {
    if m.type_ == type_ {
        Ok(())
    } else {
        Err(MailboxError::Unexpected(format!(
            "expected {type_}, got {}",
            m.type_
        )))
    }
}

fn problem(m: &MailboxMessage) -> MailboxError {
    MailboxError::Problem {
        code: str_of(&m.body, "code"),
        comment: str_of(&m.body, "comment"),
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

struct LiveFrames {
    client: std::sync::Weak<TspMailboxClient>,
    vid: String,
    wake: Waker,
}

impl FrameSink for LiveFrames {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        if let Some(c) = self.client.upgrade() {
            c.on_frame(&self.vid, frame, &self.wake);
        }
        Ok(())
    }

    fn closed(&self) {
        if let Some(c) = self.client.upgrade() {
            c.on_closed(&self.vid);
        }
    }
}

impl PickupSource for TspMailboxClient {
    /// Messages pushed on live sessions first, then a `deliver` per registered recipient that
    /// has no live session (a live session pushes everything held when it opens and every
    /// message as it arrives).
    fn fetch(&self, recipients: &[String], limit: usize) -> Result<Vec<StoredMessage>, NetError> {
        let mut out: Vec<StoredMessage> = {
            let s = lock(&self.state);
            s.pushed
                .iter()
                .filter(|m| recipients.contains(&m.recipient))
                .take(limit)
                .cloned()
                .collect()
        };
        let registered: Vec<String> = lock(&self.state).actors.keys().cloned().collect();
        let mut first_error = None;
        for r in recipients
            .iter()
            .filter(|r| registered.contains(r) && !self.is_live(r))
        {
            if out.len() >= limit {
                break;
            }
            match self.deliver(r, limit - out.len()) {
                Ok(batch) => {
                    let mut s = lock(&self.state);
                    for h in batch.messages {
                        if out.iter().any(|m| m.id == h.id) {
                            continue;
                        }
                        s.issued.insert(h.id.clone(), r.clone());
                        out.push(StoredMessage {
                            id: h.id,
                            recipient: r.clone(),
                            bytes: h.data,
                        });
                    }
                }
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) if out.is_empty() => Err(e.into()),
            _ => Ok(out),
        }
    }

    /// `received` per recipient, over the live session when one is open, else as a request.
    fn ack(&self, ids: &[String]) -> Result<(), NetError> {
        let mut by_recipient: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let s = lock(&self.state);
            for id in ids {
                if let Some(r) = s.issued.get(id) {
                    by_recipient.entry(r.clone()).or_default().push(id.clone());
                }
            }
        }
        for (r, ids) in by_recipient {
            let sent_live = self.live.as_ref().is_some_and(|l| {
                let up = lock(&l.sessions).get(&r).cloned();
                match (up, self.actor(&r)) {
                    (Some(up), Ok(actor)) => {
                        let m = MailboxMessage::request(RECEIVED, json!({ "ids": ids }));
                        seal_message(actor.identity(), &self.mediator, &m)
                            .ok()
                            .is_some_and(|b| up.send(&b).is_ok())
                    }
                    _ => false,
                }
            });
            if !sent_live {
                self.received(&r, &ids)?;
            }
            let mut s = lock(&self.state);
            for id in &ids {
                s.issued.remove(id);
            }
            s.pushed.retain(|p| !ids.contains(&p.id));
        }
        Ok(())
    }

    fn live(&self, recipients: &[String], wake: Waker) -> bool {
        if self.live.is_none() {
            return false;
        }
        let registered: Vec<String> = lock(&self.state).actors.keys().cloned().collect();
        let mut any = false;
        for r in recipients.iter().filter(|r| registered.contains(r)) {
            if self.is_live(r) || self.open_live(r, wake.clone()).is_ok() {
                any = true;
            }
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_messages_round_trip_as_json() {
        let req = MailboxMessage::request(DELIVER, json!({"limit": 5}));
        let reply = MailboxMessage::reply(
            &req,
            BATCH,
            Batch {
                messages: vec![HeldMessage {
                    id: "m1".into(),
                    size: 3,
                    arrival_ms: 7,
                    data: b"abc".to_vec(),
                }],
                remaining: 2,
                expired: ExpiryReport {
                    count: 1,
                    oldest_expiry_ms: 9,
                },
            }
            .to_json(),
        );
        let text = serde_json::to_string(&reply).unwrap();
        let back: MailboxMessage = serde_json::from_str(&text).unwrap();
        assert_eq!(back.thid.as_deref(), Some(req.id.as_str()));
        let b = Batch::from_json(&back.body).unwrap();
        assert_eq!(b.messages[0].data, b"abc");
        assert_eq!((b.remaining, b.expired.count), (2, 1));
        let st = MailboxStatus {
            message_count: 4,
            total_bytes: 40,
            oldest_age_ms: 400,
            expired: ExpiryReport::default(),
        };
        assert_eq!(MailboxStatus::from_json(&st.to_json()), st);
    }

    #[test]
    fn types_live_under_the_versioned_namespace() {
        for t in [
            REGISTER,
            REGISTERED,
            UNREGISTER,
            UNREGISTERED,
            STATUS,
            DELIVER,
            BATCH,
            RECEIVED,
            LIVE,
            PROBLEM,
        ] {
            assert!(t.starts_with(MAILBOX_NS), "{t}");
        }
    }
}
