//! The mediator: a TSP 1.0 routed-mode intermediary that keeps durable mailboxes for the VIDs
//! registered with it and serves the mailbox protocol of
//! [`vertep_federation::tsp_mailbox`] over their relationships, by request and on live sessions.
//!
//! A message addressed to the mediator is one of:
//!
//! - a relationship request, accepted unless the sender is revoked or not allowed;
//! - a routed message: the mediator opens its own layer ([`forward_routed`]). A middle hop goes
//!   on to the next intermediary. At the last hop the nested message, sealed to the recipient
//!   and opaque here, is pushed on a live session, delivered to the recipient's endpoint when it
//!   is reachable, or stored durably for pickup, and only then accepted from the previous hop;
//! - a mailbox request (`register`, `status`, `deliver`, `received`, ...), answered in the
//!   response.
//!
//! The mediator never sees an event: it handles routing data and sealed bytes only.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use serde_json::{Value, json};
use tsp_sdk::VerifiedVid;
use tsp_sdk::definitions::{Digest, Payload, RelationshipForm};
use vertep_federation::live::{FrameSink, LiveEndpoint};
use vertep_federation::net::{NetError, RelayEndpoint, RelayNetwork};
use vertep_federation::tsp::{TspIdentity, Vid, forward_routed, tsp_uri, vid_from_document};
use vertep_federation::tsp_mailbox::{
    self, BATCH, Batch, DELIVER, LIVE, MailboxMessage, Opened, PayloadKind, RECEIVED, REGISTER,
    REGISTERED, STATUS, UNREGISTER, UNREGISTERED, envelope_ids, open_message, seal_message,
};
use vertep_federation::{ActorIdentity, Clock, Did, DidDocument, DidResolver, KeyManager};

use crate::store::{MailboxStore, MailboxStoreError};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Per-VID limits of a mailbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most messages a mailbox holds; more are refused.
    pub max_messages: u64,
    /// The most bytes a mailbox holds; more are refused.
    pub max_bytes: u64,
    /// How long a message waits before it expires, in milliseconds.
    pub retention_ms: i64,
    /// The most messages in one `batch`.
    pub max_batch: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_messages: 10_000,
            max_bytes: 64 * 1024 * 1024,
            retention_ms: 14 * 24 * 3600 * 1000,
            max_batch: 100,
        }
    }
}

/// Who may register a mailbox.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RegistrationPolicy {
    /// Any VID that forms a relationship.
    #[default]
    Open,
    /// Only these VIDs.
    Allow(BTreeSet<String>),
}

/// How the mediator behaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediatorOptions {
    /// Mailbox limits.
    pub limits: Limits,
    /// Registration policy.
    pub policy: RegistrationPolicy,
    /// At the last hop, try the recipient's own endpoint before storing.
    pub direct_delivery: bool,
}

impl Default for MediatorOptions {
    fn default() -> Self {
        MediatorOptions {
            limits: Limits::default(),
            policy: RegistrationPolicy::Open,
            direct_delivery: true,
        }
    }
}

/// Why the mediator could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediatorError {
    /// The identity has no TSP endpoint or its keys are missing.
    Identity(String),
    /// The store failed.
    Store(MailboxStoreError),
}

impl fmt::Display for MediatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediatorError::Identity(m) => write!(f, "mediator identity: {m}"),
            MediatorError::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MediatorError {}

impl From<MailboxStoreError> for MediatorError {
    fn from(e: MailboxStoreError) -> Self {
        MediatorError::Store(e)
    }
}

/// Counters since start.
#[derive(Debug, Default)]
struct Counters {
    stored: AtomicU64,
    duplicates: AtomicU64,
    forwarded: AtomicU64,
    delivered_direct: AtomicU64,
    pushed: AtomicU64,
    acknowledged: AtomicU64,
    expired: AtomicU64,
    refused: AtomicU64,
}

/// One mailbox's figures in [`Metrics`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxMetrics {
    /// The recipient VID.
    pub vid: String,
    /// Waiting messages.
    pub waiting: u64,
    /// Their bytes.
    pub bytes: u64,
    /// The oldest's age in milliseconds.
    pub oldest_age_ms: i64,
    /// Messages refused for this mailbox (full, over size) since start.
    pub refusals: u64,
    /// Open live sessions.
    pub live_sessions: usize,
}

/// The mediator's metrics: per mailbox, and totals since start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    /// Registered mailboxes.
    pub mailboxes: Vec<MailboxMetrics>,
    /// Messages stored.
    pub stored: u64,
    /// Copies already held, not stored again.
    pub duplicates: u64,
    /// Routed messages forwarded to a next intermediary.
    pub forwarded: u64,
    /// Last-hop messages delivered to the recipient's endpoint.
    pub delivered_direct: u64,
    /// Messages pushed on live sessions.
    pub pushed: u64,
    /// Messages acknowledged and deleted.
    pub acknowledged: u64,
    /// Messages expired.
    pub expired: u64,
    /// Messages and requests refused.
    pub refused: u64,
}

impl Metrics {
    /// The metrics in the Prometheus text format; mailboxes are labelled by VID only.
    pub fn to_prometheus(&self) -> String {
        let mut s = String::new();
        for (name, v) in [
            ("vt_mediator_stored_total", self.stored),
            ("vt_mediator_duplicates_total", self.duplicates),
            ("vt_mediator_forwarded_total", self.forwarded),
            ("vt_mediator_delivered_direct_total", self.delivered_direct),
            ("vt_mediator_pushed_total", self.pushed),
            ("vt_mediator_acknowledged_total", self.acknowledged),
            ("vt_mediator_expired_total", self.expired),
            ("vt_mediator_refused_total", self.refused),
        ] {
            s.push_str(&format!("# TYPE {name} counter\n{name} {v}\n"));
        }
        s.push_str(&format!(
            "# TYPE vt_mediator_mailboxes gauge\nvt_mediator_mailboxes {}\n",
            self.mailboxes.len()
        ));
        for (name, kind) in [
            ("vt_mediator_mailbox_waiting", "gauge"),
            ("vt_mediator_mailbox_bytes", "gauge"),
            ("vt_mediator_mailbox_oldest_age_ms", "gauge"),
            ("vt_mediator_mailbox_refusals_total", "counter"),
        ] {
            s.push_str(&format!("# TYPE {name} {kind}\n"));
            for m in &self.mailboxes {
                let v = match name {
                    "vt_mediator_mailbox_waiting" => m.waiting.to_string(),
                    "vt_mediator_mailbox_bytes" => m.bytes.to_string(),
                    "vt_mediator_mailbox_oldest_age_ms" => m.oldest_age_ms.to_string(),
                    _ => m.refusals.to_string(),
                };
                s.push_str(&format!("{name}{{vid=\"{}\"}} {v}\n", escape(&m.vid)));
            }
        }
        s
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// A TSP mailbox mediator.
// @tg: implements:: [[openspec:tsp-mediator#TSP Routed-Mode Intermediary]]
// @tg: implements:: [[openspec:tsp-mediator#Durable Store-And-Forward]]
// @tg: implements:: [[openspec:tsp-mediator#Metadata Minimisation]] {note: "only routing data and sealed bytes reach the store"}
pub struct Mediator {
    identity: ActorIdentity,
    km: Arc<dyn KeyManager>,
    me: TspIdentity,
    /// VIDs (short form) with an established relationship.
    relationships: Mutex<BTreeSet<String>>,
    endpoint: String,
    resolver: Arc<dyn DidResolver>,
    net: Arc<dyn RelayNetwork>,
    store: MailboxStore,
    clock: Arc<dyn Clock>,
    options: MediatorOptions,
    live: Mutex<HashMap<String, Vec<Arc<dyn FrameSink>>>>,
    refusals: Mutex<BTreeMap<String, u64>>,
    counters: Counters,
    weak: Weak<Mediator>,
}

impl fmt::Debug for Mediator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mediator")
            .field("did", &self.identity.did)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// The short form of a VID: a long-form (introduction) `did:peer:4`, which carries its encoded
/// document after a second colon, without that document; any other VID as it is.
pub fn short_vid(vid: &str) -> &str {
    match vid.strip_prefix("did:peer:4") {
        Some(rest) => match rest.find(':') {
            Some(i) => &vid[..10 + i],
            None => vid,
        },
        None => vid,
    }
}

/// A refusal of the previous hop, as an answer to a post.
fn refused(m: impl Into<String>) -> NetError {
    NetError::Refused(m.into())
}

impl Mediator {
    /// A mediator with `identity` (its keys in `km`; its `TSPTransport` service is the endpoint
    /// it serves), resolving VIDs with `resolver`, posting onward through `net` and keeping
    /// mailboxes in `store`.
    ///
    /// # Errors
    ///
    /// [`MediatorError::Identity`] when the identity has no TSP endpoint or its keys are
    /// missing.
    pub fn new(
        identity: ActorIdentity,
        km: Arc<dyn KeyManager>,
        resolver: Arc<dyn DidResolver>,
        net: Arc<dyn RelayNetwork>,
        store: MailboxStore,
        clock: Arc<dyn Clock>,
        options: MediatorOptions,
    ) -> Result<Arc<Self>, MediatorError> {
        let doc = identity
            .document(km.as_ref())
            .map_err(|e| MediatorError::Identity(e.to_string()))?;
        let endpoint = tsp_uri(&doc)
            .ok_or_else(|| MediatorError::Identity(format!("{}: no TSPTransport", identity.did)))?;
        let url = url::Url::parse(&endpoint)
            .map_err(|e| MediatorError::Identity(format!("{endpoint}: {e}")))?;
        let me = TspIdentity::new(&identity, km.clone(), url)
            .map_err(|e| MediatorError::Identity(e.to_string()))?;
        Ok(Arc::new_cyclic(|weak| Mediator {
            relationships: Mutex::new(BTreeSet::new()),
            me,
            identity,
            km,
            endpoint,
            resolver,
            net,
            store,
            clock,
            options,
            live: Mutex::new(HashMap::new()),
            refusals: Mutex::new(BTreeMap::new()),
            counters: Counters::default(),
            weak: weak.clone(),
        }))
    }

    /// The mediator's DID, which is its VID.
    pub fn did(&self) -> &str {
        &self.identity.did
    }

    /// The endpoint it serves.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Its DID document, to publish.
    ///
    /// # Errors
    ///
    /// [`MediatorError::Identity`] when the keys are missing.
    pub fn document(&self) -> Result<DidDocument, MediatorError> {
        self.identity
            .document(self.km.as_ref())
            .map_err(|e| MediatorError::Identity(e.to_string()))
    }

    /// The mailboxes.
    pub fn store(&self) -> &MailboxStore {
        &self.store
    }

    fn now(&self) -> i64 {
        self.clock.now().0
    }

    fn resolve(&self, id: &str) -> Result<Vid, String> {
        if id == self.identity.did {
            return Ok(self.me.vid().clone());
        }
        let did = Did::parse(id).map_err(|e| e.to_string())?;
        let doc = self
            .resolver
            .resolve(&did)
            .map_err(|e| e.to_string())?
            .document;
        vid_from_document(&doc).map_err(|e| e.to_string())
    }

    fn refuse_count(&self, vid: &str) {
        self.counters.refused.fetch_add(1, Ordering::Relaxed);
        *lock(&self.refusals).entry(vid.to_owned()).or_default() += 1;
    }

    /// Revokes `vid`'s mailbox: what it holds is dropped, its live sessions close, and it can
    /// register again only after [`Mediator::reinstate`].
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the store fails.
    // @tg: implements:: [[openspec:tsp-mediator#Mailbox Registration]] {note: "operator revocation"}
    pub fn revoke(&self, vid: &str) -> Result<(), MailboxStoreError> {
        self.store.unregister(vid, true)?;
        self.close_live(vid);
        Ok(())
    }

    /// Lets a revoked VID register again.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the store fails.
    pub fn reinstate(&self, vid: &str) -> Result<(), MailboxStoreError> {
        if self.store.is_revoked(vid) {
            self.store.unregister(vid, false)?;
        }
        Ok(())
    }

    fn close_live(&self, vid: &str) {
        let sinks = lock(&self.live).remove(vid).unwrap_or_default();
        for s in sinks {
            s.closed();
        }
    }

    /// Expires messages past the retention period. Returns how many expired.
    ///
    /// # Errors
    ///
    /// [`MailboxStoreError`] when the store fails.
    // @tg: implements:: [[openspec:tsp-mediator#Retention And Quotas]]
    pub fn expire(&self) -> Result<usize, MailboxStoreError> {
        let n: usize = self
            .store
            .expire(self.options.limits.retention_ms, self.now())?
            .values()
            .sum();
        self.counters.expired.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    /// The metrics.
    // @tg: implements:: [[openspec:tsp-mediator#Mediator Operability]] {note: "health and per-mailbox metrics"}
    pub fn metrics(&self) -> Metrics {
        let now = self.now();
        let refusals = lock(&self.refusals).clone();
        let live = lock(&self.live);
        let c = &self.counters;
        Metrics {
            mailboxes: self
                .store
                .figures(now)
                .into_iter()
                .filter(|f| f.registered)
                .map(|f| MailboxMetrics {
                    waiting: f.status.message_count,
                    bytes: f.status.total_bytes,
                    oldest_age_ms: f.status.oldest_age_ms,
                    refusals: refusals.get(&f.vid).copied().unwrap_or(0),
                    live_sessions: live.get(&f.vid).map_or(0, Vec::len),
                    vid: f.vid,
                })
                .collect(),
            stored: c.stored.load(Ordering::Relaxed),
            duplicates: c.duplicates.load(Ordering::Relaxed),
            forwarded: c.forwarded.load(Ordering::Relaxed),
            delivered_direct: c.delivered_direct.load(Ordering::Relaxed),
            pushed: c.pushed.load(Ordering::Relaxed),
            acknowledged: c.acknowledged.load(Ordering::Relaxed),
            expired: c.expired.load(Ordering::Relaxed),
            refused: c.refused.load(Ordering::Relaxed),
        }
    }

    /// Handles one posted TSP message and returns the response body: a relationship reply, a
    /// mailbox reply, or nothing for an accepted routed message.
    ///
    /// # Errors
    ///
    /// [`NetError::Refused`] when the message is refused (the previous hop must not retry);
    /// [`NetError::Unreachable`] when it could not be stored (the previous hop keeps it and
    /// retries).
    pub fn handle(&self, bytes: &[u8]) -> Result<Vec<u8>, NetError> {
        let (sender, receiver) =
            envelope_ids(bytes).ok_or_else(|| refused("not an encrypted TSP message"))?;
        if receiver != self.identity.did {
            return Err(refused(format!("{receiver} is not this mediator")));
        }
        let opened = open_message(&self.me, bytes, &|id| {
            self.resolve(id)
                .map_err(vertep_federation::tsp::TspError::NoVid)
        })
        .map_err(|e| refused(e.to_string()))?;
        match opened {
            Opened::Mailbox { sender, message } => {
                let reply = self.answer(&sender, &message);
                self.seal_to(&sender, &reply)
                    .map_err(|e| refused(format!("reply to {sender}: {e}")))
            }
            Opened::Other(PayloadKind::Relationship) => self.relationship(&sender, bytes),
            Opened::Other(PayloadKind::Routed) => self.route(bytes),
            Opened::Other(k) => {
                self.counters.refused.fetch_add(1, Ordering::Relaxed);
                Err(refused(format!(
                    "a mediator does not accept {k:?} messages"
                )))
            }
        }
    }

    fn seal_to(&self, vid: &str, m: &MailboxMessage) -> Result<Vec<u8>, String> {
        let peer = self.resolve(vid)?;
        seal_message(&self.me, &peer, m).map_err(|e| e.to_string())
    }

    fn may_register(&self, vid: &str) -> bool {
        !self.store.is_revoked(vid)
            && match &self.options.policy {
                RegistrationPolicy::Open => true,
                RegistrationPolicy::Allow(set) => set.contains(vid),
            }
    }

    /// Answers a relationship control message (TSP 1.0 section 7): a request is accepted
    /// (`TSP_RFA`) unless the sender is revoked or not allowed (`TSP_RFD`); a cancel ends the
    /// relationship. Relationships are kept by the short form of the VID, and replies to a
    /// `did:peer:4` introduction go to its short form, as the reference implementation expects.
    fn relationship(&self, sender: &str, bytes: &[u8]) -> Result<Vec<u8>, NetError> {
        let sender_vid = self.resolve(sender).map_err(refused)?;
        let mut copy = bytes.to_vec();
        let (payload, _, _) = tsp_sdk::crypto::open(&self.me, &sender_vid, &mut copy)
            .map_err(|e| refused(e.to_string()))?;
        let short = short_vid(sender);
        let peer = if short == sender {
            sender_vid
        } else {
            Vid::new(
                short.to_owned(),
                sender_vid.endpoint().clone(),
                sender_vid.signature_key_type(),
                sender_vid.verifying_key().clone(),
                sender_vid.encryption_key_type(),
                sender_vid.encryption_key().clone(),
            )
        };
        let reply = match payload {
            Payload::RequestRelationship { thread_id, .. } => {
                if self.may_register(short) {
                    lock(&self.relationships).insert(short.to_owned());
                    Payload::AcceptRelationship {
                        thread_id,
                        reply_thread_id: Digest::default(),
                        form: RelationshipForm::Direct,
                    }
                } else {
                    self.refuse_count(short);
                    Payload::CancelRelationship { thread_id }
                }
            }
            Payload::CancelRelationship { .. } => {
                lock(&self.relationships).remove(short);
                return Ok(Vec::new());
            }
            _ => return Ok(Vec::new()),
        };
        tsp_sdk::crypto::seal_and_hash(&self.me, &peer, reply, None)
            .map_err(|e| refused(e.to_string()))
    }

    /// Opens the routed layer and forwards, delivers, pushes or stores.
    fn route(&self, bytes: &[u8]) -> Result<Vec<u8>, NetError> {
        let mut copy = bytes.to_vec();
        let hop = forward_routed(&self.me, &mut copy, self.resolver.as_ref())
            .map_err(|e| refused(e.to_string()))?;
        if !hop.last {
            let uri = self.endpoint_of(&hop.next).map_err(refused)?;
            let r = self.net.post(&uri, &hop.bytes);
            if r.is_ok() {
                self.counters.forwarded.fetch_add(1, Ordering::Relaxed);
            }
            return r;
        }
        let recipient = hop.next;
        if !self.store.is_registered(&recipient) {
            if self.options.direct_delivery && self.deliver_direct(&recipient, &hop.bytes) {
                return Ok(Vec::new());
            }
            self.counters.refused.fetch_add(1, Ordering::Relaxed);
            return Err(refused(format!(
                "{recipient} has no mailbox here and is not reachable"
            )));
        }
        let live = lock(&self.live)
            .get(&recipient)
            .is_some_and(|v| !v.is_empty());
        if !live && self.options.direct_delivery && self.deliver_direct(&recipient, &hop.bytes) {
            return Ok(Vec::new());
        }
        self.store_for(&recipient, crate::store::message_id(bytes), hop.bytes)?;
        Ok(Vec::new())
    }

    fn endpoint_of(&self, vid: &str) -> Result<String, String> {
        let did = Did::parse(vid).map_err(|e| e.to_string())?;
        let doc = self
            .resolver
            .resolve(&did)
            .map_err(|e| e.to_string())?
            .document;
        tsp_uri(&doc).ok_or_else(|| format!("{vid}: no TSP endpoint"))
    }

    /// Posts the nested message to the recipient's own endpoint; false when it is not
    /// reachable (or is this mediator).
    fn deliver_direct(&self, recipient: &str, bytes: &[u8]) -> bool {
        let Ok(uri) = self.endpoint_of(recipient) else {
            return false;
        };
        if uri == self.endpoint || !(uri.starts_with("http://") || uri.starts_with("https://")) {
            return false;
        }
        match self.net.post(&uri, bytes) {
            Ok(_) => {
                self.counters
                    .delivered_direct
                    .fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(_) => false,
        }
    }

    /// Stores for `recipient` within its limits, then pushes to its live sessions.
    fn store_for(&self, recipient: &str, id: String, bytes: Vec<u8>) -> Result<(), NetError> {
        let limits = self.options.limits;
        let st = self.store.status(recipient, self.now());
        if st.message_count >= limits.max_messages {
            self.refuse_count(recipient);
            return Err(refused(format!("mailbox of {recipient} is full")));
        }
        if st.total_bytes + bytes.len() as u64 > limits.max_bytes {
            self.refuse_count(recipient);
            return Err(refused(format!(
                "mailbox of {recipient} is over its size limit"
            )));
        }
        let stored = self
            .store
            .store_as(recipient, id, bytes, self.now())
            .map_err(|e| NetError::Unreachable(e.to_string()))?;
        if stored.duplicate {
            self.counters.duplicates.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        self.counters.stored.fetch_add(1, Ordering::Relaxed);
        if let Some(m) = self.store.get(recipient, &stored.id) {
            self.push(
                recipient,
                Batch {
                    messages: vec![m],
                    remaining: 0,
                    expired: Default::default(),
                },
            );
        }
        Ok(())
    }

    /// Pushes `batch` to every live session of `vid`, dropping sessions that fail.
    fn push(&self, vid: &str, batch: Batch) {
        let sinks = lock(&self.live).get(vid).cloned().unwrap_or_default();
        if sinks.is_empty() || batch.messages.is_empty() {
            return;
        }
        let n = batch.messages.len() as u64;
        let msg = MailboxMessage::request(BATCH, batch.to_json());
        let Ok(frame) = self.seal_to(vid, &msg) else {
            return;
        };
        let mut failed = Vec::new();
        for (i, s) in sinks.iter().enumerate() {
            if s.send(&frame).is_ok() {
                self.counters.pushed.fetch_add(n, Ordering::Relaxed);
            } else {
                failed.push(i);
            }
        }
        if !failed.is_empty()
            && let Some(v) = lock(&self.live).get_mut(vid)
        {
            let dead: Vec<Arc<dyn FrameSink>> = failed
                .iter()
                .filter_map(|i| sinks.get(*i).cloned())
                .collect();
            v.retain(|s| !dead.iter().any(|d| Arc::ptr_eq(d, s)));
        }
    }

    fn batch(&self, vid: &str, limit: usize) -> Result<Batch, MailboxStoreError> {
        let _ = self.expire();
        let limit = limit.clamp(1, self.options.limits.max_batch.max(1));
        let messages = self.store.held(vid, limit);
        let waiting = self.store.status(vid, self.now()).message_count;
        let expired = self.store.take_expiry_report(vid)?;
        Ok(Batch {
            remaining: waiting.saturating_sub(messages.len() as u64),
            messages,
            expired,
        })
    }

    /// Answers one mailbox request from the authenticated `sender`.
    // @tg: implements:: [[openspec:tsp-mediator#Pickup Protocol]]
    // @tg: implements:: [[openspec:tsp-mediator#Mailbox Registration]]
    fn answer(&self, sender: &str, m: &MailboxMessage) -> MailboxMessage {
        let problem = |code: &str, comment: &str| MailboxMessage::problem(Some(m), code, comment);
        if self.store.is_revoked(sender) {
            self.refuse_count(sender);
            return problem("revoked", "this VID's mailbox was revoked by the operator");
        }
        if m.type_ == REGISTER {
            // A relationship is required to register; afterwards the registration (which is
            // durable) stands for it, so requests keep working across mediator restarts.
            if !lock(&self.relationships).contains(short_vid(sender)) {
                return problem("no-relationship", "form a TSP relationship first");
            }
            if !self.may_register(sender) {
                self.refuse_count(sender);
                return problem("not-allowed", "registration is not open to this VID");
            }
            if let Err(e) = self.store.register(sender) {
                return problem("store", &e.to_string());
            }
            return MailboxMessage::reply(
                m,
                REGISTERED,
                json!({
                    "recipient": sender,
                    "mediator": self.identity.did,
                    "endpoint": self.endpoint,
                    "routing": [self.identity.did],
                }),
            );
        }
        if !self.store.is_registered(sender) {
            return problem("not-registered", "register a mailbox first");
        }
        if let Some(owner) = m.body.get("recipient").and_then(Value::as_str)
            && owner != sender
        {
            self.refuse_count(sender);
            return problem("not-owner", "only the recipient picks up its messages");
        }
        let now = self.now();
        match m.type_.as_str() {
            UNREGISTER => match self.store.unregister(sender, false) {
                Ok(()) => {
                    self.close_live(sender);
                    MailboxMessage::reply(m, UNREGISTERED, json!({ "recipient": sender }))
                }
                Err(e) => problem("store", &e.to_string()),
            },
            STATUS => {
                let _ = self.expire();
                let mut st = self.store.status(sender, now);
                match self.store.take_expiry_report(sender) {
                    Ok(r) => st.expired = r,
                    Err(e) => return problem("store", &e.to_string()),
                }
                MailboxMessage::reply(m, STATUS, st.to_json())
            }
            DELIVER => {
                let limit = m
                    .body
                    .get("limit")
                    .and_then(Value::as_u64)
                    .and_then(|l| usize::try_from(l).ok())
                    .unwrap_or(self.options.limits.max_batch);
                match self.batch(sender, limit) {
                    Ok(b) => MailboxMessage::reply(m, BATCH, b.to_json()),
                    Err(e) => problem("store", &e.to_string()),
                }
            }
            RECEIVED => match self.acknowledge(sender, &m.body) {
                Ok(()) => MailboxMessage::reply(
                    m,
                    STATUS,
                    self.store.status(sender, self.now()).to_json(),
                ),
                Err(e) => problem("store", &e.to_string()),
            },
            LIVE => problem(
                "use-live-endpoint",
                "live sessions open on the live endpoint",
            ),
            other => problem("unsupported", &format!("unsupported message type {other}")),
        }
    }

    fn acknowledge(&self, sender: &str, body: &Value) -> Result<(), MailboxStoreError> {
        let ids: Vec<String> = body
            .get("ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let n = self.store.received(sender, &ids)?;
        self.counters
            .acknowledged
            .fetch_add(n as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Handles one frame of a live session; returns the VID once the session is open.
    fn live_frame(&self, session: &LiveSession, frame: &[u8]) -> Result<(), NetError> {
        let opened = open_message(&self.me, frame, &|id| {
            self.resolve(id)
                .map_err(vertep_federation::tsp::TspError::NoVid)
        })
        .map_err(|e| refused(e.to_string()))?;
        let Opened::Mailbox { sender, message } = opened else {
            return Err(refused("live sessions carry mailbox messages only"));
        };
        match session.vid.get() {
            None => {
                if message.type_ != LIVE {
                    return Err(refused("a live session starts with live"));
                }
                let reply = self.answer_live_open(&sender, &message);
                if let Some(r) = reply {
                    if let Ok(f) = self.seal_to(&sender, &r) {
                        let _ = session.to_client.send(&f);
                    }
                    return Err(refused("live session refused"));
                }
                let _ = session.vid.set(sender.clone());
                lock(&self.live)
                    .entry(sender.clone())
                    .or_default()
                    .push(session.to_client.clone());
                if let Ok(b) = self.batch(&sender, self.options.limits.max_batch) {
                    self.push(&sender, b);
                }
                Ok(())
            }
            Some(vid) if *vid == sender => {
                match message.type_.as_str() {
                    RECEIVED => {
                        self.acknowledge(&sender, &message.body)
                            .map_err(|e| NetError::Unreachable(e.to_string()))?;
                    }
                    DELIVER => {
                        if let Ok(b) = self.batch(&sender, self.options.limits.max_batch) {
                            self.push(&sender, b);
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            Some(_) => Err(refused("frame from another VID")),
        }
    }

    /// `None` when `sender` may open a live session, else the problem to send.
    fn answer_live_open(&self, sender: &str, m: &MailboxMessage) -> Option<MailboxMessage> {
        if self.store.is_revoked(sender) {
            return Some(MailboxMessage::problem(
                Some(m),
                "revoked",
                "mailbox revoked",
            ));
        }
        if !self.store.is_registered(sender) {
            return Some(MailboxMessage::problem(
                Some(m),
                "not-registered",
                "register a mailbox first",
            ));
        }
        None
    }

    fn live_closed(&self, session: &LiveSession) {
        if let Some(vid) = session.vid.get()
            && let Some(v) = lock(&self.live).get_mut(vid)
        {
            v.retain(|s| !Arc::ptr_eq(s, &session.to_client));
        }
    }
}

impl RelayEndpoint for Mediator {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        self.handle(body)
    }
}

/// The mediator's side of one live session.
struct LiveSession {
    mediator: Weak<Mediator>,
    to_client: Arc<dyn FrameSink>,
    vid: OnceLock<String>,
}

impl FrameSink for LiveSession {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        match self.mediator.upgrade() {
            Some(m) => m.live_frame(self, frame),
            None => Err(NetError::Unreachable("mediator stopped".into())),
        }
    }

    fn closed(&self) {
        if let Some(m) = self.mediator.upgrade() {
            m.live_closed(self);
        }
    }
}

// @tg: implements:: [[openspec:tsp-mediator#Live Delivery Session]]
impl LiveEndpoint for Mediator {
    fn accept(&self, to_client: Arc<dyn FrameSink>) -> Arc<dyn FrameSink> {
        Arc::new(LiveSession {
            mediator: self.weak.clone(),
            to_client,
            vid: OnceLock::new(),
        })
    }
}

/// Re-exported protocol constants and types, for convenience.
pub use tsp_mailbox::{MailboxStatus, Registration};

#[cfg(test)]
mod tests {
    use super::short_vid;

    #[test]
    fn long_form_peer_vids_shorten() {
        assert_eq!(short_vid("did:peer:4zQmA:z8hhG"), "did:peer:4zQmA");
        assert_eq!(short_vid("did:peer:4zQmA"), "did:peer:4zQmA");
        assert_eq!(
            short_vid("did:peer:2.Vz6Mk.Ez6LS"),
            "did:peer:2.Vz6Mk.Ez6LS"
        );
        assert_eq!(short_vid("did:web:m.example"), "did:web:m.example");
    }
}
