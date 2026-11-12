//! The DIDComm v2 wire binding of a mediator mailbox (feature `didcomm`): Mediator Coordination
//! 2.0 (`mediate-request`, `keylist-update`) and Message Pickup 3.0 (`status-request`,
//! `delivery-request`, `messages-received`), so an actor's mailbox can live at any mediator that
//! speaks these protocols.
//!
//! [`MediatorClient`] authcrypts each request from the actor to the mediator with
//! `return_route: "all"`, posts it to the mediator's DIDComm endpoint and reads the reply from the
//! response body. A reply counts only when it is authcrypted by the mediator's own DID and
//! threads to the request. The client is a [`PickupSource`], so a
//! [`crate::inbox::PickupBinding`] picks up from a remote mediator exactly as from the in-process
//! test mediator: a delivery batch is not an acknowledgement, and `messages-received` is sent
//! only when the runtime acknowledges after the journal append.
//!
//! Senders reach the actor through the mediator with ordinary forward wrapping
//! ([`crate::didcomm::wrap_forwards`]): the actor's DIDComm service names the mediator's
//! endpoint and its key-agreement key as the routing key.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::did::{Did, DidDocument};
use crate::didcomm::{DidCommAgent, DidCommError, jwe_recipients};
use crate::live::{FrameSink, LiveNetwork};
use crate::net::{NetError, PickupSource, RelayNetwork, StoredMessage};
use crate::resolver::DidResolver;
use crate::time::Clock;

/// `coordinate-mediation/2.0/mediate-request`.
pub const MEDIATE_REQUEST: &str = "https://didcomm.org/coordinate-mediation/2.0/mediate-request";
/// `coordinate-mediation/2.0/mediate-grant`.
pub const MEDIATE_GRANT: &str = "https://didcomm.org/coordinate-mediation/2.0/mediate-grant";
/// `coordinate-mediation/2.0/mediate-deny`.
pub const MEDIATE_DENY: &str = "https://didcomm.org/coordinate-mediation/2.0/mediate-deny";
/// `coordinate-mediation/2.0/keylist-update`.
pub const KEYLIST_UPDATE: &str = "https://didcomm.org/coordinate-mediation/2.0/keylist-update";
/// `coordinate-mediation/2.0/keylist-update-response`.
pub const KEYLIST_UPDATE_RESPONSE: &str =
    "https://didcomm.org/coordinate-mediation/2.0/keylist-update-response";
/// `messagepickup/3.0/status-request`.
pub const STATUS_REQUEST: &str = "https://didcomm.org/messagepickup/3.0/status-request";
/// `messagepickup/3.0/status`.
pub const STATUS: &str = "https://didcomm.org/messagepickup/3.0/status";
/// `messagepickup/3.0/delivery-request`.
pub const DELIVERY_REQUEST: &str = "https://didcomm.org/messagepickup/3.0/delivery-request";
/// `messagepickup/3.0/delivery`.
pub const DELIVERY: &str = "https://didcomm.org/messagepickup/3.0/delivery";
/// `messagepickup/3.0/messages-received`.
pub const MESSAGES_RECEIVED: &str = "https://didcomm.org/messagepickup/3.0/messages-received";
/// `messagepickup/3.0/live-delivery-change`: turns live delivery on or off for the session.
pub const LIVE_DELIVERY_CHANGE: &str = "https://didcomm.org/messagepickup/3.0/live-delivery-change";
/// `report-problem/2.0/problem-report`.
pub const PROBLEM_REPORT: &str = "https://didcomm.org/report-problem/2.0/problem-report";

/// Why a mediator exchange failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediatorError {
    /// The mediator could not be reached.
    Net(NetError),
    /// A request could not be sealed or a reply could not be opened.
    DidComm(DidCommError),
    /// The mediator has no usable DIDComm endpoint or key in its DID document.
    NoEndpoint(String),
    /// The mediator answered with a problem report.
    Problem {
        /// The problem code (`e.p.req.not_enrolled`, ...).
        code: String,
        /// The human-readable comment, if any.
        comment: String,
    },
    /// The mediator denied mediation.
    Denied,
    /// The reply was not authenticated by the mediator, did not thread to the request, or was
    /// not the expected message.
    Unexpected(String),
}

impl fmt::Display for MediatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediatorError::Net(e) => write!(f, "mediator: {e}"),
            MediatorError::DidComm(e) => write!(f, "mediator: {e}"),
            MediatorError::NoEndpoint(m) => write!(f, "mediator has no DIDComm endpoint: {m}"),
            MediatorError::Problem { code, comment } => {
                write!(f, "mediator problem report {code}: {comment}")
            }
            MediatorError::Denied => f.write_str("mediation denied"),
            MediatorError::Unexpected(m) => write!(f, "unexpected mediator reply: {m}"),
        }
    }
}

impl std::error::Error for MediatorError {}

impl From<NetError> for MediatorError {
    fn from(e: NetError) -> Self {
        MediatorError::Net(e)
    }
}

impl From<DidCommError> for MediatorError {
    fn from(e: DidCommError) -> Self {
        MediatorError::DidComm(e)
    }
}

impl From<MediatorError> for NetError {
    fn from(e: MediatorError) -> Self {
        match e {
            MediatorError::Net(n) => n,
            other => NetError::Refused(other.to_string()),
        }
    }
}

/// The plaintext of a DIDComm protocol message from `from` to `to`, asking for the reply on the
/// same connection (`return_route: "all"`).
pub fn protocol_message(id: &str, type_: &str, from: &str, to: &str, body: Value) -> Value {
    json!({
        "id": id,
        "type": type_,
        "from": from,
        "to": [to],
        "created_time": crate::time::Timestamp::now().0 / 1000,
        "return_route": "all",
        "body": body,
    })
}

/// The bytes of a `delivery` attachment: `data.base64` (standard or URL-safe), or `data.json`
/// re-serialized.
pub fn attachment_bytes(attachment: &Value) -> Option<Vec<u8>> {
    let data = attachment.get("data")?;
    if let Some(b) = data.get("base64").and_then(Value::as_str) {
        let b = b.trim_end_matches('=');
        return URL_SAFE_NO_PAD.decode(b).ok().or_else(|| {
            STANDARD
                .decode(format!("{b}{}", "=".repeat((4 - b.len() % 4) % 4)))
                .ok()
        });
    }
    data.get("json").map(|j| j.to_string().into_bytes())
}

fn new_id() -> String {
    let mut b = [0u8; 16];
    // A failing system RNG leaves zeros; the id is a label, not a secret.
    let _ = getrandom::fill(&mut b);
    // Version 4 UUID layout, which some mediators expect of message ids.
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// One actor's (or one shard's) session with a DIDComm v2 mediator: mediation, its recipient
/// keylist, and message pickup over the Pickup 3.0 wire messages.
// @tg: implements:: [[openspec:relay-mailboxes#DIDComm Message Pickup]]
// @tg: implements:: [[openspec:relay-mailboxes#DIDComm Mediator Mailbox]]
pub struct MediatorClient {
    agent: DidCommAgent,
    mediator: DidDocument,
    endpoint: String,
    net: Arc<dyn RelayNetwork>,
    resolver: Arc<dyn DidResolver>,
    clock: Arc<dyn Clock>,
    live: Option<Arc<DidCommLive>>,
}

/// The live-delivery side of a [`MediatorClient`]: the WebSocket (or in-process) session and the
/// messages the mediator pushed on it, held until the runtime acknowledges them.
struct DidCommLive {
    network: Arc<dyn LiveNetwork>,
    uri: String,
    pushed: Arc<Mutex<VecDeque<StoredMessage>>>,
    up: Mutex<Option<Arc<dyn FrameSink>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Frames from the mediator on a live session: `delivery` messages are opened, checked to come
/// from the mediator, and their attachments held for the next fetch.
struct DeliveryFrames {
    agent: DidCommAgent,
    mediator: String,
    resolver: Arc<dyn DidResolver>,
    clock: Arc<dyn Clock>,
    pushed: Arc<Mutex<VecDeque<StoredMessage>>>,
    wake: Arc<dyn Fn(&str) + Send + Sync>,
    live: std::sync::Weak<DidCommLive>,
}

impl FrameSink for DeliveryFrames {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        let Ok(text) = std::str::from_utf8(frame) else {
            return Ok(());
        };
        let Ok(plain) = self
            .agent
            .decrypt(text, self.resolver.as_ref(), self.clock.now())
        else {
            return Ok(());
        };
        if plain.sender.as_ref().map(|(d, _)| d.as_str()) != Some(self.mediator.as_str()) {
            return Ok(());
        }
        let m = plain.message;
        if m.get("type").and_then(Value::as_str) != Some(DELIVERY) {
            return Ok(());
        }
        let asked = m
            .pointer("/body/recipient_did")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut woken = Vec::new();
        {
            let mut pushed = lock(&self.pushed);
            for a in m
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let (Some(id), Some(bytes)) =
                    (a.get("id").and_then(Value::as_str), attachment_bytes(a))
                else {
                    continue;
                };
                if pushed.iter().any(|p| p.id == id) {
                    continue;
                }
                let recipient = asked.clone().unwrap_or_else(|| {
                    std::str::from_utf8(&bytes)
                        .map(jwe_recipients)
                        .unwrap_or_default()
                        .into_iter()
                        .next()
                        .unwrap_or_default()
                });
                woken.push(recipient.clone());
                pushed.push_back(StoredMessage {
                    id: id.to_owned(),
                    recipient,
                    bytes,
                });
            }
        }
        woken.dedup();
        for r in woken {
            (self.wake)(&r);
        }
        Ok(())
    }

    fn closed(&self) {
        if let Some(l) = self.live.upgrade() {
            *lock(&l.up) = None;
        }
    }
}

impl fmt::Debug for MediatorClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediatorClient")
            .field("actor", &self.agent.identity().did)
            .field("mediator", &self.mediator.id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl MediatorClient {
    /// A session of `agent` with the mediator `mediator_did`, resolved with `resolver`; requests
    /// go to the first DIDComm service endpoint of its document.
    ///
    /// # Errors
    ///
    /// [`MediatorError::NoEndpoint`] when the mediator does not resolve or has no DIDComm
    /// endpoint.
    pub fn new(
        agent: DidCommAgent,
        mediator_did: &str,
        net: Arc<dyn RelayNetwork>,
        resolver: Arc<dyn DidResolver>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, MediatorError> {
        let did = Did::parse(mediator_did)
            .map_err(|e| MediatorError::NoEndpoint(format!("{mediator_did}: {e}")))?;
        let mediator = resolver
            .resolve(&did)
            .map_err(|e| MediatorError::NoEndpoint(format!("{mediator_did}: {e}")))?
            .document;
        let endpoint = mediator
            .service
            .iter()
            .filter(|s| s.has_type("DIDCommMessaging"))
            .flat_map(|s| s.endpoints.iter())
            .map(|e| e.uri.clone())
            .find(|u| u.starts_with("http"))
            .ok_or_else(|| MediatorError::NoEndpoint(mediator_did.to_owned()))?;
        Ok(MediatorClient {
            agent,
            mediator,
            endpoint,
            net,
            resolver,
            clock,
            live: None,
        })
    }

    /// This client with live delivery (Pickup 3.0 `live-delivery-change`) over `network`, at the
    /// mediator's live endpoint ([`crate::live::live_uri`] of its DIDComm endpoint).
    #[must_use]
    pub fn with_live(mut self, network: Arc<dyn LiveNetwork>) -> Self {
        self.live = Some(Arc::new(DidCommLive {
            uri: crate::live::live_uri(&self.endpoint),
            network,
            pushed: Arc::new(Mutex::new(VecDeque::new())),
            up: Mutex::new(None),
        }));
        self
    }

    /// True when a live session is open.
    pub fn is_live(&self) -> bool {
        self.live.as_ref().is_some_and(|l| lock(&l.up).is_some())
    }

    /// Opens a live session and asks the mediator to push deliveries on it
    /// (`live-delivery-change` with `live_delivery: true`).
    ///
    /// # Errors
    ///
    /// [`MediatorError`] when no live network is configured, the connection fails or the
    /// request cannot be sealed.
    pub fn go_live(&self, wake: Arc<dyn Fn(&str) + Send + Sync>) -> Result<(), MediatorError> {
        let Some(live) = &self.live else {
            return Err(MediatorError::Unexpected("no live network".into()));
        };
        let frames = Arc::new(DeliveryFrames {
            agent: self.agent.clone(),
            mediator: self.mediator.id.clone(),
            resolver: self.resolver.clone(),
            clock: self.clock.clone(),
            pushed: live.pushed.clone(),
            wake,
            live: Arc::downgrade(live),
        });
        let up = live.network.connect(&live.uri, frames)?;
        let msg = protocol_message(
            &new_id(),
            LIVE_DELIVERY_CHANGE,
            &self.agent.identity().did,
            &self.mediator.id,
            json!({ "live_delivery": true }),
        );
        let jwe = self.agent.seal_message(&msg, &self.mediator)?;
        up.send(jwe.as_bytes())?;
        *lock(&live.up) = Some(up);
        Ok(())
    }

    /// The mediator's DID.
    pub fn mediator_did(&self) -> &str {
        &self.mediator.id
    }

    /// The mediator's endpoint requests are posted to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The routing key a recipient lists in its DIDComm service so senders forward through this
    /// mediator: the mediator's first key-agreement key.
    pub fn routing_key(&self) -> Option<String> {
        self.mediator.key_agreement.first().cloned()
    }

    /// Sends one request and returns the mediator's reply, authenticated as the mediator's and
    /// threaded to the request. A problem report becomes [`MediatorError::Problem`].
    ///
    /// # Errors
    ///
    /// [`MediatorError`] for network, sealing and opening failures and unexpected replies.
    pub fn exchange(&self, type_: &str, body: Value) -> Result<Value, MediatorError> {
        self.exchange_optional(type_, body)?.ok_or_else(|| {
            MediatorError::Unexpected(format!("no reply to {type_} on the request's connection"))
        })
    }

    /// Like [`MediatorClient::exchange`], for requests the mediator may leave unanswered
    /// (`None` when the response body is empty).
    ///
    /// # Errors
    ///
    /// [`MediatorError`] for network, sealing and opening failures and unexpected replies.
    pub fn exchange_optional(
        &self,
        type_: &str,
        body: Value,
    ) -> Result<Option<Value>, MediatorError> {
        let id = new_id();
        let msg = protocol_message(
            &id,
            type_,
            &self.agent.identity().did,
            &self.mediator.id,
            body,
        );
        let jwe = self.agent.seal_message(&msg, &self.mediator)?;
        let resp = self.net.post(&self.endpoint, jwe.as_bytes())?;
        if resp.iter().all(u8::is_ascii_whitespace) {
            return Ok(None);
        }
        let text = std::str::from_utf8(&resp)
            .map_err(|e| MediatorError::Unexpected(format!("reply is not text: {e}")))?;
        let reply = self
            .agent
            .decrypt(text, self.resolver.as_ref(), self.clock.now())?;
        match &reply.sender {
            Some((did, _)) if did.as_str() == self.mediator.id => {}
            other => {
                return Err(MediatorError::Unexpected(format!(
                    "reply to {type_} not authenticated by the mediator (sender {:?})",
                    other.as_ref().map(|(d, _)| d.to_string())
                )));
            }
        }
        let m = reply.message;
        let rtype = m.get("type").and_then(Value::as_str).unwrap_or_default();
        if rtype == PROBLEM_REPORT {
            return Err(MediatorError::Problem {
                code: m
                    .pointer("/body/code")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                comment: m
                    .pointer("/body/comment")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        if m.get("thid").and_then(Value::as_str) != Some(id.as_str()) {
            return Err(MediatorError::Unexpected(format!(
                "{rtype} does not thread to {type_} {id}"
            )));
        }
        Ok(Some(m))
    }

    fn expect(&self, m: &Value, types: &[&str]) -> Result<(), MediatorError> {
        let t = m.get("type").and_then(Value::as_str).unwrap_or_default();
        if types.contains(&t) {
            Ok(())
        } else {
            Err(MediatorError::Unexpected(format!(
                "expected {types:?}, got {t}"
            )))
        }
    }

    /// Asks the mediator to mediate for this actor (`mediate-request`) and returns its routing
    /// DID(s) from `mediate-grant`.
    ///
    /// # Errors
    ///
    /// [`MediatorError::Denied`] on `mediate-deny`, other [`MediatorError`]s otherwise.
    pub fn request_mediation(&self) -> Result<Vec<String>, MediatorError> {
        let m = self.exchange(MEDIATE_REQUEST, json!({}))?;
        if m.get("type").and_then(Value::as_str) == Some(MEDIATE_DENY) {
            return Err(MediatorError::Denied);
        }
        self.expect(&m, &[MEDIATE_GRANT])?;
        Ok(match m.pointer("/body/routing_did") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        })
    }

    /// Adds recipient DIDs to this actor's keylist at the mediator (`keylist-update`), so the
    /// mediator accepts forwards for them.
    ///
    /// # Errors
    ///
    /// [`MediatorError`] when the mediator refuses an update.
    pub fn add_recipients(&self, dids: &[String]) -> Result<(), MediatorError> {
        let updates: Vec<Value> = dids
            .iter()
            .map(|d| json!({"recipient_did": d, "action": "add"}))
            .collect();
        let m = self.exchange(KEYLIST_UPDATE, json!({ "updates": updates }))?;
        self.expect(&m, &[KEYLIST_UPDATE_RESPONSE])?;
        for u in m
            .pointer("/body/updated")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let result = u.get("result").and_then(Value::as_str).unwrap_or_default();
            if !matches!(result, "success" | "no_change") {
                return Err(MediatorError::Problem {
                    code: result.to_owned(),
                    comment: format!(
                        "keylist update for {}",
                        u.get("recipient_did")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                    ),
                });
            }
        }
        Ok(())
    }

    /// The number of messages the mediator holds (`status-request`), for `recipient` or for all
    /// of this actor's recipients.
    ///
    /// # Errors
    ///
    /// [`MediatorError`] for failed exchanges.
    pub fn status(&self, recipient: Option<&str>) -> Result<u64, MediatorError> {
        let body = match recipient {
            Some(r) => json!({ "recipient_did": r }),
            None => json!({}),
        };
        let m = self.exchange(STATUS_REQUEST, body)?;
        self.expect(&m, &[STATUS])?;
        Ok(m.pointer("/body/message_count")
            .and_then(Value::as_u64)
            .unwrap_or(0))
    }

    /// Up to `limit` held messages (`delivery-request`), for `recipient` or for all of this
    /// actor's recipients. The messages stay held until [`MediatorClient::messages_received`].
    ///
    /// # Errors
    ///
    /// [`MediatorError`] for failed exchanges.
    pub fn delivery(
        &self,
        recipient: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>, MediatorError> {
        let mut body = json!({ "limit": limit });
        if let Some(r) = recipient {
            body["recipient_did"] = json!(r);
        }
        let m = self.exchange(DELIVERY_REQUEST, body)?;
        // No messages: the mediator answers with a status instead of an empty delivery.
        self.expect(&m, &[DELIVERY, STATUS])?;
        let mut out = Vec::new();
        for a in m
            .get("attachments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = a
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| MediatorError::Unexpected("delivery attachment without id".into()))?
                .to_owned();
            let bytes = attachment_bytes(a).ok_or_else(|| {
                MediatorError::Unexpected(format!("delivery attachment {id} has no data"))
            })?;
            let addressed = std::str::from_utf8(&bytes)
                .map(jwe_recipients)
                .unwrap_or_default();
            let recipient = recipient
                .map(str::to_owned)
                .or_else(|| addressed.into_iter().next())
                .unwrap_or_default();
            out.push(StoredMessage {
                id,
                recipient,
                bytes,
            });
        }
        Ok(out)
    }

    /// Tells the mediator it may delete the messages `ids` (`messages-received`). Returns the
    /// count it still holds when it answers with a `status`; Pickup 3.0 lets it stay silent
    /// (`None`), as the Identus mediator does.
    ///
    /// # Errors
    ///
    /// [`MediatorError`] for failed exchanges.
    pub fn messages_received(&self, ids: &[String]) -> Result<Option<u64>, MediatorError> {
        let Some(m) =
            self.exchange_optional(MESSAGES_RECEIVED, json!({ "message_id_list": ids }))?
        else {
            return Ok(None);
        };
        self.expect(&m, &[STATUS])?;
        Ok(m.pointer("/body/message_count").and_then(Value::as_u64))
    }
}

// @tg: implements:: [[openspec:relay-mailboxes#Live Delivery Over WebSocket]] {note: "DIDComm Pickup 3.0 live mode"}
impl PickupSource for MediatorClient {
    fn fetch(&self, recipients: &[String], limit: usize) -> Result<Vec<StoredMessage>, NetError> {
        let mut out: Vec<StoredMessage> = self
            .live
            .as_ref()
            .map(|l| {
                lock(&l.pushed)
                    .iter()
                    .filter(|m| recipients.contains(&m.recipient))
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !out.is_empty() {
            return Ok(out);
        }
        for r in recipients {
            if out.len() >= limit {
                break;
            }
            out.extend(self.delivery(Some(r), limit - out.len())?);
        }
        Ok(out)
    }

    fn ack(&self, ids: &[String]) -> Result<(), NetError> {
        if !ids.is_empty() {
            self.messages_received(ids)?;
        }
        if let Some(l) = &self.live {
            lock(&l.pushed).retain(|p| !ids.contains(&p.id));
        }
        Ok(())
    }

    fn live(&self, _recipients: &[String], wake: Arc<dyn Fn(&str) + Send + Sync>) -> bool {
        self.live.is_some() && (self.is_live() || self.go_live(wake).is_ok())
    }
}
