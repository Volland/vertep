//! Live delivery sessions: a bidirectional frame channel between a mailbox and its relay, over
//! which the relay pushes messages as they arrive and the mailbox sends acknowledgements.
//!
//! The seam is three small traits. A relay that offers live sessions is a [`LiveEndpoint`]; a
//! mailbox reaches it through a [`LiveNetwork`]; each side writes to the other through a
//! [`FrameSink`]. [`LocalLiveNetwork`] connects in-process endpoints (tests and single-process
//! deployments) and can drop connections to simulate a lost socket; a WebSocket network plugs in
//! at the same traits (`vertep-tsp-mediator` has one).
//!
//! A live session never changes acknowledgement: a pushed message stays at the relay until the
//! mailbox acknowledges it after the journal append, so a dropped connection loses nothing.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use crate::net::NetError;

/// One direction of a live session: frames written here reach the other side.
pub trait FrameSink: Send + Sync {
    /// Delivers one frame.
    ///
    /// # Errors
    ///
    /// [`NetError::Unreachable`] when the session is closed.
    fn send(&self, frame: &[u8]) -> Result<(), NetError>;

    /// The other side closed the session (or the connection dropped).
    fn closed(&self) {}
}

/// A relay that accepts live sessions.
pub trait LiveEndpoint: Send + Sync {
    /// A client connected: `to_client` writes frames to it. Returns the sink of the frames the
    /// client sends.
    fn accept(&self, to_client: Arc<dyn FrameSink>) -> Arc<dyn FrameSink>;
}

/// How a mailbox opens live sessions with relays.
pub trait LiveNetwork: Send + Sync {
    /// Connects to the live endpoint at `uri`; frames from the relay go to `to_client`.
    /// Returns the sink of frames to the relay.
    ///
    /// # Errors
    ///
    /// [`NetError`] when the endpoint is unknown or unreachable.
    fn connect(
        &self,
        uri: &str,
        to_client: Arc<dyn FrameSink>,
    ) -> Result<Arc<dyn FrameSink>, NetError>;
}

/// A fresh random label for a frame or message id (hex).
pub fn frame_id() -> String {
    let mut b = [0u8; 12];
    // A failing system RNG leaves zeros; the id is a label, not a secret.
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The live endpoint URI that goes with a relay's HTTP endpoint: the scheme becomes `ws` or
/// `wss` and `/live` is appended (`https://m.example/tsp` → `wss://m.example/tsp/live`).
pub fn live_uri(endpoint: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    let base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_owned()
    };
    format!("{base}/live")
}

struct Connection {
    uri: String,
    open: AtomicBool,
    to_client: Arc<dyn FrameSink>,
    to_server: OnceLock<Arc<dyn FrameSink>>,
}

impl Connection {
    fn close(&self) {
        if self.open.swap(false, Ordering::SeqCst) {
            self.to_client.closed();
            if let Some(s) = self.to_server.get() {
                s.closed();
            }
        }
    }
}

struct ClientEnd(Arc<Connection>);

impl FrameSink for ClientEnd {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        if !self.0.open.load(Ordering::SeqCst) {
            return Err(NetError::Unreachable(format!(
                "{}: session closed",
                self.0.uri
            )));
        }
        self.0.to_client.send(frame)
    }
    fn closed(&self) {
        self.0.close();
    }
}

struct ServerEnd(Arc<Connection>);

impl FrameSink for ServerEnd {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        if !self.0.open.load(Ordering::SeqCst) {
            return Err(NetError::Unreachable(format!(
                "{}: session closed",
                self.0.uri
            )));
        }
        match self.0.to_server.get() {
            Some(s) => s.send(frame),
            None => Err(NetError::Unreachable(format!(
                "{}: not accepted",
                self.0.uri
            ))),
        }
    }
    fn closed(&self) {
        self.0.close();
    }
}

/// An in-process live network: URIs map to [`LiveEndpoint`]s.
#[derive(Default)]
pub struct LocalLiveNetwork {
    endpoints: RwLock<BTreeMap<String, Arc<dyn LiveEndpoint>>>,
    connections: Mutex<Vec<Weak<Connection>>>,
}

impl fmt::Debug for LocalLiveNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let eps: Vec<String> = self
            .endpoints
            .read()
            .map(|e| e.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("LocalLiveNetwork")
            .field("endpoints", &eps)
            .finish()
    }
}

impl LocalLiveNetwork {
    /// An empty network.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Serves `endpoint` at `uri`.
    pub fn serve(&self, uri: &str, endpoint: Arc<dyn LiveEndpoint>) {
        if let Ok(mut e) = self.endpoints.write() {
            e.insert(uri.to_owned(), endpoint);
        }
    }

    /// Stops serving `uri` and drops its open connections.
    pub fn remove(&self, uri: &str) {
        if let Ok(mut e) = self.endpoints.write() {
            e.remove(uri);
        }
        self.drop_connections(uri);
    }

    /// Drops every open connection to `uri`, as a lost socket does: both sides see the session
    /// close and later frames fail.
    pub fn drop_connections(&self, uri: &str) {
        let conns: Vec<Arc<Connection>> = self
            .connections
            .lock()
            .map(|c| c.iter().filter_map(Weak::upgrade).collect())
            .unwrap_or_default();
        for c in conns.into_iter().filter(|c| c.uri == uri) {
            c.close();
        }
    }

    /// The number of open connections to `uri`.
    pub fn open_connections(&self, uri: &str) -> usize {
        self.connections
            .lock()
            .map(|c| {
                c.iter()
                    .filter_map(Weak::upgrade)
                    .filter(|c| c.uri == uri && c.open.load(Ordering::SeqCst))
                    .count()
            })
            .unwrap_or(0)
    }
}

impl LiveNetwork for LocalLiveNetwork {
    fn connect(
        &self,
        uri: &str,
        to_client: Arc<dyn FrameSink>,
    ) -> Result<Arc<dyn FrameSink>, NetError> {
        let ep = self
            .endpoints
            .read()
            .ok()
            .and_then(|e| e.get(uri).cloned())
            .ok_or_else(|| NetError::Gone(uri.to_owned()))?;
        let conn = Arc::new(Connection {
            uri: uri.to_owned(),
            open: AtomicBool::new(true),
            to_client,
            to_server: OnceLock::new(),
        });
        if let Ok(mut c) = self.connections.lock() {
            c.retain(|w| w.strong_count() > 0);
            c.push(Arc::downgrade(&conn));
        }
        let server = ep.accept(Arc::new(ClientEnd(conn.clone())));
        let _ = conn.to_server.set(server);
        Ok(Arc::new(ServerEnd(conn)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl LiveEndpoint for Echo {
        fn accept(&self, to_client: Arc<dyn FrameSink>) -> Arc<dyn FrameSink> {
            struct Back(Arc<dyn FrameSink>);
            impl FrameSink for Back {
                fn send(&self, frame: &[u8]) -> Result<(), NetError> {
                    self.0.send(frame)
                }
            }
            Arc::new(Back(to_client))
        }
    }

    #[derive(Default)]
    struct Collect(Mutex<Vec<Vec<u8>>>, AtomicBool);
    impl FrameSink for Collect {
        fn send(&self, frame: &[u8]) -> Result<(), NetError> {
            self.0.lock().unwrap().push(frame.to_vec());
            Ok(())
        }
        fn closed(&self) {
            self.1.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn frames_flow_both_ways_until_the_connection_drops() {
        let net = LocalLiveNetwork::new();
        net.serve("ws://m/live", Arc::new(Echo));
        let got = Arc::new(Collect::default());
        let up = net.connect("ws://m/live", got.clone()).unwrap();
        up.send(b"hello").unwrap();
        assert_eq!(got.0.lock().unwrap().as_slice(), &[b"hello".to_vec()]);
        assert_eq!(net.open_connections("ws://m/live"), 1);
        net.drop_connections("ws://m/live");
        assert!(got.1.load(Ordering::SeqCst), "the client saw the close");
        assert!(up.send(b"again").is_err());
        assert_eq!(net.open_connections("ws://m/live"), 0);
    }

    #[test]
    fn live_uris_follow_the_http_endpoint() {
        assert_eq!(
            live_uri("https://m.example/tsp"),
            "wss://m.example/tsp/live"
        );
        assert_eq!(live_uri("http://m:8700/tsp/"), "ws://m:8700/tsp/live");
    }
}
