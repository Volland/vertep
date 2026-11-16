//! Live delivery over WebSocket (feature `ws`): the socket binding of the [`crate::live`] seam.
//!
//! WebSocket is a transport of relay mailboxes, not a second protocol. A frame on the socket is
//! exactly a frame of a live session: a sealed mailbox message (a DIDComm Pickup 3.0 `delivery`
//! or `messages-received`, a TSP mailbox batch or `received`). Acknowledgement is unchanged: a
//! pushed message stays at the relay until the mailbox acknowledges it after the journal append,
//! so a socket that drops loses nothing, and the pickup loop ([`crate::pickup_loop`]) falls
//! back to polling until the socket is back.
//!
//! - [`WsLiveNetwork`] is the client: a [`LiveNetwork`] that opens `ws://` sessions.
//! - [`WsLiveServer`] puts [`LiveEndpoint`]s on a socket by request path: the test mediator of
//!   `testkit`, or a relay that runs in one process. It can refuse upgrades and drop every open
//!   socket, which is how the tests take the socket away.
//!
//! Both speak plain `ws://`; put TLS in front with a terminating proxy.
//!
//! @tg: implements:: [[decisions#WebSocket is a mailbox transport]]

use std::collections::BTreeMap;
use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http;

use crate::live::{FrameSink, LiveEndpoint, LiveNetwork};
use crate::net::NetError;

/// Frames queued for one socket; `None` closes it.
type Outgoing = mpsc::UnboundedSender<Option<Vec<u8>>>;

/// The write half of a socket as a [`FrameSink`]: frames are queued for the socket's task.
struct SocketSink {
    tx: Outgoing,
}

impl FrameSink for SocketSink {
    fn send(&self, frame: &[u8]) -> Result<(), NetError> {
        self.tx
            .send(Some(frame.to_vec()))
            .map_err(|_| NetError::Unreachable("live socket closed".into()))
    }

    fn closed(&self) {
        let _ = self.tx.send(None);
    }
}

fn frame_of(m: Message) -> Option<Vec<u8>> {
    match m {
        Message::Binary(b) => Some(b.to_vec()),
        Message::Text(t) => Some(t.as_bytes().to_vec()),
        _ => None,
    }
}

/// A WebSocket client for live sessions with relays. Each session runs on its own thread with a
/// small tokio runtime; frames from the relay reach the session's sink on that thread. When the
/// socket closes or fails, the sink sees [`FrameSink::closed`], which is what tells a mailbox
/// client to fall back to polling.
// @tg: implements:: [[openspec:relay-mailboxes#WebSocket Live Delivery]] {note: "the client end of the socket"}
#[derive(Clone, Copy, Debug)]
pub struct WsLiveNetwork {
    connect_timeout: Duration,
}

impl Default for WsLiveNetwork {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

impl WsLiveNetwork {
    /// A client that gives up opening a session after `connect_timeout`.
    pub fn new(connect_timeout: Duration) -> Self {
        WsLiveNetwork { connect_timeout }
    }
}

impl LiveNetwork for WsLiveNetwork {
    fn connect(
        &self,
        uri: &str,
        to_client: Arc<dyn FrameSink>,
    ) -> Result<Arc<dyn FrameSink>, NetError> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), NetError>>();
        let uri = uri.to_owned();
        std::thread::Builder::new()
            .name("vt-ws-live".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(NetError::Unreachable(e.to_string())));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ws = match tokio_tungstenite::connect_async(uri.as_str()).await {
                        Ok((ws, _)) => ws,
                        Err(e) => {
                            let _ =
                                ready_tx.send(Err(NetError::Unreachable(format!("{uri}: {e}"))));
                            return;
                        }
                    };
                    if ready_tx.send(Ok(())).is_err() {
                        // The caller timed out and went on without this session.
                        return;
                    }
                    let (mut write, mut read) = ws.split();
                    loop {
                        tokio::select! {
                            out = rx.recv() => match out {
                                Some(Some(frame)) => {
                                    if write.send(Message::Binary(frame.into())).await.is_err() {
                                        break;
                                    }
                                }
                                _ => {
                                    let _ = write.close().await;
                                    break;
                                }
                            },
                            incoming = read.next() => match incoming {
                                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                                Some(Ok(m)) => {
                                    if let Some(frame) = frame_of(m) {
                                        let _ = to_client.send(&frame);
                                    }
                                }
                            },
                        }
                    }
                    to_client.closed();
                });
            })
            .map_err(|e| NetError::Unreachable(e.to_string()))?;
        ready_rx
            .recv_timeout(self.connect_timeout)
            .map_err(|_| NetError::Unreachable("live connect timed out".into()))??;
        Ok(Arc::new(SocketSink { tx }))
    }
}

struct Shared {
    routes: RwLock<BTreeMap<String, Arc<dyn LiveEndpoint>>>,
    refusing: AtomicBool,
    kill: watch::Sender<u64>,
    open: AtomicUsize,
}

impl Shared {
    fn route(&self, path: &str) -> Option<Arc<dyn LiveEndpoint>> {
        self.routes.read().ok().and_then(|r| r.get(path).cloned())
    }
}

fn refusal(status: u16, why: &str) -> ErrorResponse {
    let mut r = http::Response::new(Some(why.to_owned()));
    *r.status_mut() = http::StatusCode::from_u16(status).unwrap_or(http::StatusCode::NOT_FOUND);
    r
}

/// A WebSocket server for [`LiveEndpoint`]s, routed by request path, on its own thread.
///
/// It serves what [`crate::live::LocalLiveNetwork`] serves in-process, over a real socket:
/// `serve("/didcomm/live", mediator)` answers `ws://<addr>/didcomm/live`, the
/// [`crate::live::live_uri`] of a relay whose HTTP endpoint is `http://<addr>/didcomm`. Dropping
/// the server closes every socket.
pub struct WsLiveServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for WsLiveServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let paths: Vec<String> = self
            .shared
            .routes
            .read()
            .map(|r| r.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("WsLiveServer")
            .field("addr", &self.addr)
            .field("paths", &paths)
            .finish()
    }
}

impl WsLiveServer {
    /// Binds `addr` (`127.0.0.1:0` picks a free port) and starts accepting sockets.
    ///
    /// # Errors
    ///
    /// The I/O error when the address cannot be bound or the server thread cannot start.
    pub fn bind(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let (kill, _) = watch::channel(0u64);
        let shared = Arc::new(Shared {
            routes: RwLock::new(BTreeMap::new()),
            refusing: AtomicBool::new(false),
            kill,
            open: AtomicUsize::new(0),
        });
        let (stop, mut stopped) = oneshot::channel::<()>();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("vt-ws-server".into())
            .spawn(move || {
                rt.block_on(async move {
                    let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                        return;
                    };
                    loop {
                        tokio::select! {
                            _ = &mut stopped => break,
                            accepted = listener.accept() => {
                                if let Ok((stream, _)) = accepted {
                                    let _ = stream.set_nodelay(true);
                                    tokio::spawn(session(stream, s.clone()));
                                }
                            }
                        }
                    }
                });
                // Dropping the runtime cancels the sessions and closes their sockets.
            })?;
        Ok(WsLiveServer {
            addr,
            shared,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://<addr>`: the base of the HTTP endpoint whose [`crate::live::live_uri`] this server
    /// answers.
    pub fn http_base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Serves `endpoint` for upgrades to `path` (`/didcomm/live`).
    pub fn serve(&self, path: &str, endpoint: Arc<dyn LiveEndpoint>) {
        if let Ok(mut r) = self.shared.routes.write() {
            r.insert(path.to_owned(), endpoint);
        }
    }

    /// Stops serving `path`; later upgrades to it are answered 404.
    pub fn remove(&self, path: &str) {
        if let Ok(mut r) = self.shared.routes.write() {
            r.remove(path);
        }
    }

    /// Refuses new upgrades with 503 (`true`) or accepts them again, as a relay that turned live
    /// mode off or a proxy that drops upgrades does. Open sockets are not affected.
    pub fn set_refusing(&self, refusing: bool) {
        self.shared.refusing.store(refusing, Ordering::SeqCst);
    }

    /// Drops every open socket without a close handshake, as a lost connection does.
    pub fn drop_connections(&self) {
        self.shared.kill.send_modify(|g| *g += 1);
    }

    /// The number of open sockets.
    pub fn open_connections(&self) -> usize {
        self.shared.open.load(Ordering::SeqCst)
    }
}

impl Drop for WsLiveServer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// One socket: the handshake picks the endpoint by path, then frames flow both ways until either
/// side closes or the server drops it.
// The handshake callback's error type is tungstenite's `ErrorResponse`, an HTTP response.
#[allow(clippy::result_large_err)]
async fn session(stream: tokio::net::TcpStream, shared: Arc<Shared>) {
    let mut endpoint = None;
    let callback = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if shared.refusing.load(Ordering::SeqCst) {
            return Err(refusal(503, "live sessions refused"));
        }
        match shared.route(req.uri().path()) {
            Some(ep) => {
                endpoint = Some(ep);
                Ok(resp)
            }
            None => Err(refusal(404, "no live endpoint at this path")),
        }
    };
    let Ok(ws) = tokio_tungstenite::accept_hdr_async(stream, callback).await else {
        return;
    };
    let Some(endpoint) = endpoint else {
        return;
    };
    let mut kill = shared.kill.subscribe();
    let (tx, mut rx) = mpsc::unbounded_channel::<Option<Vec<u8>>>();
    let from_client = endpoint.accept(Arc::new(SocketSink { tx }));
    shared.open.fetch_add(1, Ordering::SeqCst);
    let (mut write, mut read) = ws.split();
    loop {
        tokio::select! {
            // Dropped by the server: no close frame, as when the connection is lost.
            _ = kill.changed() => break,
            out = rx.recv() => match out {
                Some(Some(frame)) => {
                    if write.send(Message::Binary(frame.into())).await.is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = write.close().await;
                    break;
                }
            },
            incoming = read.next() => match incoming {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(m)) => {
                    if let Some(frame) = frame_of(m) {
                        let _ = from_client.send(&frame);
                    }
                }
            },
        }
    }
    shared.open.fetch_sub(1, Ordering::SeqCst);
    from_client.closed();
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::live::live_uri;

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

    fn wait(f: impl Fn() -> bool) -> bool {
        for _ in 0..200 {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn frames_flow_over_a_socket_until_the_server_drops_it() {
        let server = WsLiveServer::bind("127.0.0.1:0").unwrap();
        server.serve("/relay/live", Arc::new(Echo));
        let uri = live_uri(&format!("{}/relay", server.http_base()));
        let got = Arc::new(Collect::default());
        let up = WsLiveNetwork::default().connect(&uri, got.clone()).unwrap();
        up.send(b"hello").unwrap();
        assert!(wait(|| got.0.lock().unwrap().len() == 1));
        assert_eq!(got.0.lock().unwrap()[0], b"hello");
        assert!(wait(|| server.open_connections() == 1));
        server.drop_connections();
        assert!(
            wait(|| got.1.load(Ordering::SeqCst)),
            "the client saw the drop"
        );
        assert!(wait(|| up.send(b"again").is_err()));
        assert!(wait(|| server.open_connections() == 0));
    }

    #[test]
    fn refused_and_unknown_upgrades_fail_to_connect() {
        let server = WsLiveServer::bind("127.0.0.1:0").unwrap();
        server.serve("/relay/live", Arc::new(Echo));
        let net = WsLiveNetwork::new(Duration::from_secs(5));
        let unknown = format!("ws://{}/other/live", server.local_addr());
        assert!(net.connect(&unknown, Arc::new(Collect::default())).is_err());
        server.set_refusing(true);
        let uri = format!("ws://{}/relay/live", server.local_addr());
        assert!(net.connect(&uri, Arc::new(Collect::default())).is_err());
        server.set_refusing(false);
        assert!(net.connect(&uri, Arc::new(Collect::default())).is_ok());
    }
}
