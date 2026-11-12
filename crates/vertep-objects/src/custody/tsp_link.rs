//! Custody over TSP (feature `tsp`): custody requests and answers as TSP application messages
//! between a host's VID and a custodian's VID, direct or through intermediaries.
//!
//! A request travels as a relay envelope named `vt.custody.request` whose data is the request's
//! JSON; the answer is `vt.custody.response` with the same correlation id. The custodian
//! checks that the TSP-authenticated sender is the party the request speaks for (the candidate
//! of a prepare, the writer of an append, a host for a read).
//!
//! Direct: the request is posted to the custodian's TSP endpoint and the sealed answer comes
//! back in the response body. Through a store-and-forward intermediary (a TSP mediator): the
//! request is routed to the mediator, which holds it; the custodian picks it up
//! ([`pump_mailbox`]), and posts its answer to the host's endpoint, where a [`ReplyInbox`]
//! hands it to the waiting request.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::envelope::RelayEnvelope;
use vertep_federation::tsp::{SealedTsp, TspError, TspReceived, TspSession, Vid};
use vertep_federation::{NetError, OpenedMessage, PickupSource, RelayEndpoint, RelayNetwork};

use super::link::{CustodyTransport, LinkError};
use super::replica::Replica;
use super::wire::{CustodyRequest, CustodyResponse};

/// The event name of a custody request.
pub const CUSTODY_REQUEST_EVENT: &str = "vt.custody.request";
/// The event name of a custody answer.
pub const CUSTODY_RESPONSE_EVENT: &str = "vt.custody.response";

/// Resolves a VID identifier (a DID) to its public VID.
pub type ResolveVid = Arc<dyn Fn(&str) -> Result<Vid, TspError> + Send + Sync>;

fn envelope(name: &str, id: &str, json: String) -> RelayEnvelope {
    let mut e = RelayEnvelope::from_event(
        format!("{id}/{name}"),
        &Event {
            name: name.into(),
            data: Value::String(json),
            ..Event::named(name, EventType::External)
        },
    );
    e.correlation = Some(id.to_owned());
    e
}

fn payload(env: &RelayEnvelope) -> Result<&str, String> {
    match &env.data.0 {
        Value::String(s) => Ok(s),
        _ => Err("custody payload is not a JSON string".into()),
    }
}

/// A custodian's TSP endpoint: opens custody requests, checks who sent them, answers them from
/// its replica and seals the answer back to the sender.
pub struct TspCustodyEndpoint {
    session: Arc<TspSession>,
    replica: Arc<Replica>,
    resolve: ResolveVid,
}

impl fmt::Debug for TspCustodyEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspCustodyEndpoint")
            .field("custodian", &self.replica.custodian())
            .finish_non_exhaustive()
    }
}

impl TspCustodyEndpoint {
    /// The endpoint of `replica` with the custodian's TSP `session`.
    pub fn new(session: Arc<TspSession>, replica: Arc<Replica>, resolve: ResolveVid) -> Self {
        TspCustodyEndpoint {
            session,
            replica,
            resolve,
        }
    }

    /// Opens one TSP message, answers it, and returns the sealed answer for the sender.
    ///
    /// # Errors
    ///
    /// Why the message was not answered: it does not open, is not a custody request, or its
    /// TSP sender is not the party the request speaks for.
    // @tg: implements:: [[openspec:object-custody#Journal Replication Over TSP]]
    pub fn process(&self, bytes: &[u8]) -> Result<SealedTsp, String> {
        match open_message(&self.session, &self.resolve, bytes)? {
            Opened::Control(sealed) => Ok(sealed),
            Opened::Event(m) => self.answer(&m),
        }
    }

    /// Answers an opened custody request.
    ///
    /// # Errors
    ///
    /// Why it was not answered.
    fn answer(&self, opened: &OpenedMessage) -> Result<SealedTsp, String> {
        if opened.envelope.name != CUSTODY_REQUEST_EVENT {
            return Err(format!("not a custody request: {}", opened.envelope.name));
        }
        let sender = opened
            .origin
            .sender
            .clone()
            .filter(|_| opened.origin.verified)
            .ok_or("unauthenticated custody request")?;
        let request: CustodyRequest =
            serde_json::from_str(payload(&opened.envelope)?).map_err(|e| e.to_string())?;
        let speaks_for = match &request {
            CustodyRequest::Prepare { candidate, .. } => candidate == &sender,
            CustodyRequest::Append(b) => b.writer == sender,
            CustodyRequest::Read { .. } => self.replica.config().is_host(&sender),
        };
        let answer = if speaks_for {
            self.replica.handle(&request)
        } else {
            CustodyResponse::Failed {
                reason: format!("{sender} does not speak for this request"),
            }
        };
        let id = opened.envelope.correlation.clone().unwrap_or_default();
        let json = serde_json::to_string(&answer).map_err(|e| e.to_string())?;
        let peer = (self.resolve)(&sender).map_err(|e| e.to_string())?;
        self.session
            .seal_event(&peer, &envelope(CUSTODY_RESPONSE_EVENT, &id, json))
            .map_err(|e| e.to_string())
    }
}

enum Opened {
    /// A relationship request, already answered.
    Control(SealedTsp),
    /// An application message.
    Event(Box<OpenedMessage>),
}

fn open_message(session: &TspSession, resolve: &ResolveVid, bytes: &[u8]) -> Result<Opened, String> {
    let mut copy = bytes.to_vec();
    let r = |id: &str| resolve(id);
    match session.open(&mut copy, &r).map_err(|e| e.to_string())? {
        TspReceived::Event(m) => Ok(Opened::Event(m)),
        TspReceived::RelationshipRequested { sender, thread_id } => {
            let peer = resolve(&sender).map_err(|e| e.to_string())?;
            session
                .accept_relationship(&peer, thread_id)
                .map(Opened::Control)
                .map_err(|e| e.to_string())
        }
        other => Err(format!("unexpected TSP message {other:?}")),
    }
}

/// One TSP endpoint for a host that is also a custodian: custody requests from other hosts are
/// answered by its replica, answers to its own routed requests go to its reply inbox.
pub struct CustodyPort {
    endpoint: TspCustodyEndpoint,
    inbox: Arc<ReplyInbox>,
}

impl fmt::Debug for CustodyPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustodyPort").finish_non_exhaustive()
    }
}

impl CustodyPort {
    /// The port of `endpoint` and `inbox` (both on the same TSP session).
    pub fn new(endpoint: TspCustodyEndpoint, inbox: Arc<ReplyInbox>) -> Self {
        CustodyPort { endpoint, inbox }
    }
}

impl RelayEndpoint for CustodyPort {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        match open_message(&self.endpoint.session, &self.endpoint.resolve, body)
            .map_err(NetError::Refused)?
        {
            Opened::Control(sealed) => Ok(sealed.bytes),
            Opened::Event(m) if m.envelope.name == CUSTODY_RESPONSE_EVENT => {
                self.inbox.deliver(&m).map_err(NetError::Refused)?;
                Ok(Vec::new())
            }
            Opened::Event(m) => self
                .endpoint
                .answer(&m)
                .map(|s| s.bytes)
                .map_err(NetError::Refused),
        }
    }
}

impl RelayEndpoint for TspCustodyEndpoint {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        self.process(body)
            .map(|s| s.bytes)
            .map_err(NetError::Refused)
    }
}

/// Picks up what `source` (a mediator) holds for `custodian`, answers each custody request and
/// posts the answers over `network`, acknowledging each message only after it was answered.
/// Returns how many were answered.
///
/// # Errors
///
/// [`NetError`] when the mediator cannot be reached.
pub fn pump_mailbox(
    endpoint: &TspCustodyEndpoint,
    source: &dyn PickupSource,
    custodian: &str,
    network: &dyn RelayNetwork,
) -> Result<usize, NetError> {
    let held = source.fetch(&[custodian.to_owned()], 64)?;
    let mut done = Vec::new();
    for m in &held {
        match endpoint.process(&m.bytes) {
            Ok(answer) => {
                if network
                    .post(answer.endpoint.as_str(), &answer.bytes)
                    .is_ok()
                {
                    done.push(m.id.clone());
                }
            }
            // A message that will never be answered is acknowledged so it is not redelivered.
            Err(_) => done.push(m.id.clone()),
        }
    }
    if !done.is_empty() {
        source.ack(&done)?;
    }
    Ok(done.len())
}

/// A host's endpoint for answers that come back asynchronously (through a mediator).
pub struct ReplyInbox {
    session: Arc<TspSession>,
    resolve: ResolveVid,
    replies: Mutex<BTreeMap<String, (String, CustodyResponse)>>,
    arrived: Condvar,
}

impl fmt::Debug for ReplyInbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplyInbox").finish_non_exhaustive()
    }
}

impl ReplyInbox {
    /// An inbox opening answers with the host's `session`.
    pub fn new(session: Arc<TspSession>, resolve: ResolveVid) -> Arc<Self> {
        Arc::new(ReplyInbox {
            session,
            resolve,
            replies: Mutex::new(BTreeMap::new()),
            arrived: Condvar::new(),
        })
    }

    fn wait(&self, id: &str, timeout: Duration) -> Option<(String, CustodyResponse)> {
        let deadline = std::time::Instant::now() + timeout;
        let mut g = self
            .replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(r) = g.remove(id) {
                return Some(r);
            }
            let left = deadline.checked_duration_since(std::time::Instant::now())?;
            g = self
                .arrived
                .wait_timeout(g, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}

fn open_answer(
    session: &TspSession,
    resolve: &ResolveVid,
    bytes: &[u8],
) -> Result<(String, String, CustodyResponse), String> {
    let mut copy = bytes.to_vec();
    let r = |id: &str| resolve(id);
    let TspReceived::Event(m) = session.open(&mut copy, &r).map_err(|e| e.to_string())? else {
        return Err("not a custody answer".into());
    };
    if m.envelope.name != CUSTODY_RESPONSE_EVENT {
        return Err(format!("not a custody answer: {}", m.envelope.name));
    }
    let sender = m.origin.sender.clone().ok_or("anonymous answer")?;
    let answer = serde_json::from_str(payload(&m.envelope)?).map_err(|e| e.to_string())?;
    Ok((
        m.envelope.correlation.clone().unwrap_or_default(),
        sender,
        answer,
    ))
}

impl ReplyInbox {
    fn store(&self, id: String, sender: String, answer: CustodyResponse) {
        self.replies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id, (sender, answer));
        self.arrived.notify_all();
    }

    fn deliver(&self, m: &OpenedMessage) -> Result<(), String> {
        let sender = m.origin.sender.clone().ok_or("anonymous answer")?;
        let answer = serde_json::from_str(payload(&m.envelope)?).map_err(|e| e.to_string())?;
        self.store(
            m.envelope.correlation.clone().unwrap_or_default(),
            sender,
            answer,
        );
        Ok(())
    }
}

impl RelayEndpoint for ReplyInbox {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        let (id, sender, answer) =
            open_answer(&self.session, &self.resolve, body).map_err(NetError::Refused)?;
        self.store(id, sender, answer);
        Ok(Vec::new())
    }
}

/// A host's [`CustodyTransport`] to one custodian over TSP.
pub struct TspCustodyTransport {
    custodian: String,
    session: Arc<TspSession>,
    network: Arc<dyn RelayNetwork>,
    resolve: ResolveVid,
    route: Vec<Vid>,
    inbox: Option<Arc<ReplyInbox>>,
    timeout: Duration,
    counter: AtomicU64,
}

impl fmt::Debug for TspCustodyTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspCustodyTransport")
            .field("custodian", &self.custodian)
            .finish_non_exhaustive()
    }
}

impl TspCustodyTransport {
    /// Direct TSP to `custodian`: the answer comes back in the response body.
    pub fn direct(
        custodian: impl Into<String>,
        session: Arc<TspSession>,
        network: Arc<dyn RelayNetwork>,
        resolve: ResolveVid,
    ) -> Self {
        TspCustodyTransport {
            custodian: custodian.into(),
            session,
            network,
            resolve,
            route: Vec::new(),
            inbox: None,
            timeout: Duration::from_secs(5),
            counter: AtomicU64::new(0),
        }
    }

    /// Routed TSP through `route` (the custodian's mediator last): answers come back to
    /// `inbox`, waiting at most `timeout`.
    pub fn routed(
        custodian: impl Into<String>,
        session: Arc<TspSession>,
        network: Arc<dyn RelayNetwork>,
        resolve: ResolveVid,
        route: Vec<Vid>,
        inbox: Arc<ReplyInbox>,
        timeout: Duration,
    ) -> Self {
        TspCustodyTransport {
            custodian: custodian.into(),
            session,
            network,
            resolve,
            route,
            inbox: Some(inbox),
            timeout,
            counter: AtomicU64::new(0),
        }
    }
}

impl CustodyTransport for TspCustodyTransport {
    fn custodian(&self) -> &str {
        &self.custodian
    }

    fn call(&self, request: &CustodyRequest) -> Result<CustodyResponse, LinkError> {
        let err = |e: &dyn fmt::Display| LinkError(e.to_string());
        let peer = (self.resolve)(&self.custodian).map_err(|e| err(&e))?;
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let mut nonce = [0u8; 8];
        let _ = getrandom::fill(&mut nonce);
        let id = format!("urn:vertep:custody:{}:{n}", u64::from_le_bytes(nonce));
        let json = serde_json::to_string(request).map_err(|e| err(&e))?;
        let env = envelope(CUSTODY_REQUEST_EVENT, &id, json);
        let (sender, answer) = if self.route.is_empty() {
            let sealed = self.session.seal_event(&peer, &env).map_err(|e| err(&e))?;
            let body = self
                .network
                .post(sealed.endpoint.as_str(), &sealed.bytes)
                .map_err(|e| err(&e))?;
            let (got, sender, answer) =
                open_answer(&self.session, &self.resolve, &body).map_err(|e| err(&e))?;
            if got != id {
                return Err(LinkError(format!("answer to {got}, not {id}")));
            }
            (sender, answer)
        } else {
            let inbox = self
                .inbox
                .as_ref()
                .ok_or_else(|| LinkError("routed custody needs a reply inbox".into()))?;
            let sealed = self
                .session
                .seal_routed(&peer, &self.route, &env)
                .map_err(|e| err(&e))?;
            self.network
                .post(sealed.endpoint.as_str(), &sealed.bytes)
                .map_err(|e| err(&e))?;
            inbox
                .wait(&id, self.timeout)
                .ok_or_else(|| LinkError(format!("no answer from {} in time", self.custodian)))?
        };
        if sender != self.custodian {
            return Err(LinkError(format!(
                "answer from {sender}, not {}",
                self.custodian
            )));
        }
        Ok(answer)
    }
}
