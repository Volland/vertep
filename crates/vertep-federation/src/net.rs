//! The network seam of relay transports, and store-and-forward boxes.
//!
//! A relay transport posts sealed bytes to an endpoint URI through a [`RelayNetwork`]; the
//! response body carries protocol replies (a TSP relationship accept). A mailbox fetches what a
//! mediator or intermediary holds through a [`PickupSource`] and acknowledges it only after the
//! receiving journal append (or a recorded rejection).
//!
//! [`LocalNetwork`] routes posts to in-process [`RelayEndpoint`]s; [`DropBox`] is the
//! store-and-forward queue behind a mediator, an intermediary's drop-off or a node's own TSP
//! endpoint. Both are deterministic and offline, which is what tests and single-process
//! deployments need; an HTTP or WebSocket network plugs in at the same traits.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// Why a post or a pickup failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// The endpoint did not answer (connection refused, timeout): worth retrying or falling back.
    Unreachable(String),
    /// The endpoint no longer exists (HTTP 404/410, unknown host): the cached document is stale.
    Gone(String),
    /// The endpoint answered and refused the message (an intermediary refusing to route).
    Refused(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Unreachable(m) => write!(f, "unreachable: {m}"),
            NetError::Gone(m) => write!(f, "endpoint gone: {m}"),
            NetError::Refused(m) => write!(f, "refused: {m}"),
        }
    }
}

impl std::error::Error for NetError {}

/// Posts sealed relay messages to endpoint URIs.
pub trait RelayNetwork: Send + Sync {
    /// Posts `body` to `endpoint` and returns the response body (empty when there is none).
    ///
    /// # Errors
    ///
    /// [`NetError`] when the endpoint is unreachable, gone or refuses the message.
    fn post(&self, endpoint: &str, body: &[u8]) -> Result<Vec<u8>, NetError>;
}

/// Something listening at an endpoint URI: a mediator, an intermediary, a node's TSP endpoint.
pub trait RelayEndpoint: Send + Sync {
    /// Handles one posted message and returns the response body.
    ///
    /// # Errors
    ///
    /// [`NetError::Refused`] when the message is refused.
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError>;
}

/// An in-process network: endpoint URIs map to [`RelayEndpoint`]s, and endpoints can be taken
/// down to simulate outages.
#[derive(Default)]
pub struct LocalNetwork {
    endpoints: RwLock<BTreeMap<String, Arc<dyn RelayEndpoint>>>,
    down: RwLock<BTreeSet<String>>,
    posts: Mutex<Vec<(String, Vec<u8>)>>,
}

impl fmt::Debug for LocalNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let eps: Vec<String> = self
            .endpoints
            .read()
            .map(|e| e.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("LocalNetwork")
            .field("endpoints", &eps)
            .finish()
    }
}

impl LocalNetwork {
    /// An empty network.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Serves `endpoint` at `uri`.
    pub fn serve(&self, uri: &str, endpoint: Arc<dyn RelayEndpoint>) {
        if let Ok(mut e) = self.endpoints.write() {
            e.insert(uri.to_owned(), endpoint);
        }
    }

    /// Removes the endpoint at `uri`: later posts fail with [`NetError::Gone`].
    pub fn remove(&self, uri: &str) {
        if let Ok(mut e) = self.endpoints.write() {
            e.remove(uri);
        }
    }

    /// Takes `uri` down (`true`) or brings it back up: posts to a down endpoint fail with
    /// [`NetError::Unreachable`].
    pub fn set_down(&self, uri: &str, down: bool) {
        if let Ok(mut d) = self.down.write() {
            if down {
                d.insert(uri.to_owned());
            } else {
                d.remove(uri);
            }
        }
    }

    /// Every post so far, as `(endpoint, body)`: what an observer of the wire saw.
    pub fn posts(&self) -> Vec<(String, Vec<u8>)> {
        lock(&self.posts).clone()
    }

    /// The number of posts to `uri`.
    pub fn posts_to(&self, uri: &str) -> usize {
        lock(&self.posts).iter().filter(|(u, _)| u == uri).count()
    }
}

impl RelayNetwork for LocalNetwork {
    fn post(&self, endpoint: &str, body: &[u8]) -> Result<Vec<u8>, NetError> {
        if self.down.read().is_ok_and(|d| d.contains(endpoint)) {
            return Err(NetError::Unreachable(endpoint.to_owned()));
        }
        let ep = self
            .endpoints
            .read()
            .ok()
            .and_then(|e| e.get(endpoint).cloned())
            .ok_or_else(|| NetError::Gone(endpoint.to_owned()))?;
        lock(&self.posts).push((endpoint.to_owned(), body.to_vec()));
        ep.receive(body)
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A message held by a mediator or intermediary for one recipient.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMessage {
    /// The relay's own id for the stored copy (what is acknowledged). Each copy has its own, so
    /// a re-sent message is a second copy with the same message id inside.
    pub id: String,
    /// The recipient DID the relay stored it for (routing data, not the event).
    pub recipient: String,
    /// The still-encrypted message.
    pub bytes: Vec<u8>,
}

/// Where a mailbox picks up stored messages: the DIDComm message pickup protocol against a
/// mediator, a TSP intermediary's drop-off, or a node's own inbound queue.
pub trait PickupSource: Send + Sync {
    /// Up to `limit` messages held for any of `recipients`, oldest first. Messages stay held
    /// until acknowledged, so the same copy can be fetched again (a delivery batch is not an
    /// acknowledgement).
    ///
    /// # Errors
    ///
    /// [`NetError`] when the relay cannot be reached.
    fn fetch(&self, recipients: &[String], limit: usize) -> Result<Vec<StoredMessage>, NetError>;

    /// Acknowledges stored copies `ids`, which the relay may then delete
    /// (`messages-received`).
    ///
    /// # Errors
    ///
    /// [`NetError`] when the relay cannot be reached; the copies are then fetched again later.
    fn ack(&self, ids: &[String]) -> Result<(), NetError>;

    /// Opens a live session: `wake` is called with the recipient whenever a message for one of
    /// `recipients` arrives, so it is delivered without waiting for the next poll. Returns
    /// false when the relay offers no live mode.
    fn live(&self, _recipients: &[String], _wake: Arc<dyn Fn(&str) + Send + Sync>) -> bool {
        false
    }
}

type Waker = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct BoxState {
    queue: VecDeque<StoredMessage>,
    next: u64,
    live: Vec<(BTreeSet<String>, Waker)>,
    fetches: usize,
    acked: Vec<String>,
}

/// A store-and-forward queue keyed by recipient: what a mediator or intermediary keeps. It
/// holds encrypted bytes and recipient DIDs only, keeps arrival order, hands copies out until
/// they are acknowledged, and wakes live sessions.
#[derive(Default)]
pub struct DropBox {
    name: String,
    state: Mutex<BoxState>,
}

impl fmt::Debug for DropBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DropBox")
            .field("name", &self.name)
            .field("held", &lock(&self.state).queue.len())
            .finish()
    }
}

impl DropBox {
    /// An empty box; `name` prefixes the ids of stored copies.
    pub fn new(name: impl Into<String>) -> Arc<Self> {
        Arc::new(DropBox {
            name: name.into(),
            state: Mutex::new(BoxState::default()),
        })
    }

    /// Stores `bytes` for `recipient` and returns the copy's id.
    pub fn store(&self, recipient: &str, bytes: Vec<u8>) -> String {
        let (id, wakers) = {
            let mut s = lock(&self.state);
            let id = format!("{}#{}", self.name, s.next);
            s.next += 1;
            s.queue.push_back(StoredMessage {
                id: id.clone(),
                recipient: recipient.to_owned(),
                bytes,
            });
            let wakers: Vec<Waker> = s
                .live
                .iter()
                .filter(|(r, _)| r.contains(recipient))
                .map(|(_, w)| w.clone())
                .collect();
            (id, wakers)
        };
        for w in wakers {
            w(recipient);
        }
        id
    }

    /// Every held copy, oldest first.
    pub fn held(&self) -> Vec<StoredMessage> {
        lock(&self.state).queue.iter().cloned().collect()
    }

    /// The copies held for `recipient`.
    pub fn held_for(&self, recipient: &str) -> Vec<StoredMessage> {
        lock(&self.state)
            .queue
            .iter()
            .filter(|m| m.recipient == recipient)
            .cloned()
            .collect()
    }

    /// Replaces the bytes of copy `id` (a relay tampering with what it stores).
    pub fn tamper(&self, id: &str, f: impl FnOnce(&mut Vec<u8>)) {
        if let Some(m) = lock(&self.state).queue.iter_mut().find(|m| m.id == id) {
            f(&mut m.bytes);
        }
    }

    /// The number of fetches served.
    pub fn fetches(&self) -> usize {
        lock(&self.state).fetches
    }

    /// The ids acknowledged so far, in order.
    pub fn acknowledged(&self) -> Vec<String> {
        lock(&self.state).acked.clone()
    }
}

impl PickupSource for DropBox {
    fn fetch(&self, recipients: &[String], limit: usize) -> Result<Vec<StoredMessage>, NetError> {
        let mut s = lock(&self.state);
        s.fetches += 1;
        Ok(s.queue
            .iter()
            .filter(|m| recipients.contains(&m.recipient))
            .take(limit)
            .cloned()
            .collect())
    }

    fn ack(&self, ids: &[String]) -> Result<(), NetError> {
        let mut s = lock(&self.state);
        s.queue.retain(|m| !ids.contains(&m.id));
        s.acked.extend(ids.iter().cloned());
        Ok(())
    }

    fn live(&self, recipients: &[String], wake: Waker) -> bool {
        lock(&self.state)
            .live
            .push((recipients.iter().cloned().collect(), wake));
        true
    }
}
