//! The server end of the WebSocket Event I/O Processor (feature `websocket`): a WebSocket
//! endpoint on tokio that browser and Node.js actor systems connect to.
//!
//! - The first frame must be `hello`. Its credential goes to the application's check; a refused
//!   credential gets a `refused` frame and the connection closes.
//! - Every client node (its `hello` node IRI) has one [`Link`] that outlives its connections:
//!   messages to the client's actors wait in it until acknowledged and are sent again when the
//!   client reconnects.
//! - A received `message` is delivered to the local actor it targets, as an external event whose
//!   `_event.origintype` is [`WEBSOCKET_PROCESSOR`] and whose `_event.origin` is the client
//!   actor's IRI, then acknowledged. A redelivered message is journaled once: the runtime drops
//!   a message id it already journaled.
//! - Server actors reach client actors by IRI: the endpoint registers a resolver for the
//!   configured IRI schemes that routes an address to the client that sent from it, or whose
//!   `hello` listed a prefix of it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::channel::mpsc::unbounded;
use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use super::{Frame, Inbound, Link, LinkMailbox, WEBSOCKET_PROCESSOR};
use crate::address::Address;
use crate::mailbox::Mailbox;
use crate::node::{DeliveryError, NodeHandle};
use crate::transport::Resolver;

/// Checks a connection's credential: `Ok` accepts it, `Err(reason)` refuses it.
pub type CredentialCheck = Arc<dyn Fn(Option<&str>) -> Result<(), String> + Send + Sync>;

/// How a [`WebSocketServer`] is set up.
#[derive(Clone)]
pub struct ServerOptions {
    /// The server node's IRI, sent in `welcome`.
    pub node: String,
    /// IRI prefixes of the server's actors, sent in `welcome`.
    pub hosts: Vec<String>,
    /// The IRI schemes of client actor addresses (the resolver is registered for each).
    pub schemes: Vec<String>,
    /// The credential check; every credential is accepted when absent.
    pub credentials: Option<CredentialCheck>,
}

impl std::fmt::Debug for ServerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerOptions")
            .field("node", &self.node)
            .field("hosts", &self.hosts)
            .field("schemes", &self.schemes)
            .finish_non_exhaustive()
    }
}

impl ServerOptions {
    /// A server named `node` that accepts every credential and resolves `http`, `https` and
    /// `urn` client addresses.
    pub fn new(node: impl Into<String>) -> Self {
        ServerOptions {
            node: node.into(),
            hosts: Vec::new(),
            schemes: vec!["http".into(), "https".into(), "urn".into()],
            credentials: None,
        }
    }

    /// With a credential check.
    pub fn credentials(
        mut self,
        f: impl Fn(Option<&str>) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.credentials = Some(Arc::new(f));
        self
    }

    /// With the prefixes of the server's actors announced in `welcome`.
    pub fn hosts(mut self, hosts: Vec<String>) -> Self {
        self.hosts = hosts;
        self
    }
}

struct Inner {
    handle: NodeHandle,
    options: ServerOptions,
    links: Mutex<BTreeMap<String, Arc<Link>>>,
    prefixes: Mutex<Vec<(String, String)>>,
    generation: AtomicU64,
    kill: tokio::sync::watch::Sender<u64>,
    hold_acks: AtomicBool,
    duplicate: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The WebSocket endpoint of a runtime node. Cloning shares it.
// @tg: implements:: [[openspec:wasm-bindings#Server Interop Over WebSocket]] {confidence: partial, note: "the server end in vertep-runtime"}
#[derive(Clone)]
pub struct WebSocketServer {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for WebSocketServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketServer")
            .field("options", &self.inner.options)
            .field("clients", &lock(&self.inner.links).len())
            .finish()
    }
}

struct ServerResolver(Arc<Inner>);

impl Resolver for ServerResolver {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        let link = self.0.link_for(address.as_str())?;
        Some(Arc::new(LinkMailbox::new(
            link,
            Some(self.0.handle.clone()),
        )))
    }
}

impl Inner {
    fn link_for(&self, iri: &str) -> Option<Arc<Link>> {
        let links = lock(&self.links);
        if let Some(l) = links.values().find(|l| l.routes_to(iri)) {
            return Some(l.clone());
        }
        let prefixes = lock(&self.prefixes);
        prefixes
            .iter()
            .filter(|(p, _)| iri.starts_with(p.as_str()))
            .max_by_key(|(p, _)| p.len())
            .and_then(|(_, node)| links.get(node).cloned())
    }

    fn inbound(&self, link: &Link, text: &str) {
        let Ok(Inbound::Deliver(env)) = link.receive(text) else {
            return;
        };
        let id = env.message_id.clone();
        match self.handle.send_envelope(env.clone()) {
            Ok(()) => {}
            Err(DeliveryError::Unresolvable(t)) => {
                // nobody to deliver to: a dead letter, acknowledged so the sender stops
                if let Ok(a) = Address::parse(&t) {
                    self.handle.record_dead_letter(&a, &env, "unresolvable");
                }
            }
            // not accepted now: unacknowledged, so the client sends it again on reconnect
            Err(_) => return,
        }
        if !self.hold_acks.load(Ordering::SeqCst) {
            link.ack(&id);
        }
    }
}

impl WebSocketServer {
    /// An endpoint delivering into the node behind `handle`; registers its resolver of client
    /// addresses for the configured schemes.
    pub fn new(handle: NodeHandle, options: ServerOptions) -> Self {
        let (kill, _) = tokio::sync::watch::channel(0);
        let inner = Arc::new(Inner {
            handle,
            options,
            links: Mutex::new(BTreeMap::new()),
            prefixes: Mutex::new(Vec::new()),
            generation: AtomicU64::new(1),
            kill,
            hold_acks: AtomicBool::new(false),
            duplicate: AtomicBool::new(false),
        });
        for s in &inner.options.schemes {
            inner
                .handle
                .register_resolver(s, Arc::new(ServerResolver(inner.clone())));
        }
        WebSocketServer { inner }
    }

    /// Accepts connections from `listener` until it fails, each on its own tokio task.
    ///
    /// # Errors
    ///
    /// The listener's accept error.
    pub async fn serve(&self, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _) = listener.accept().await?;
            let me = self.clone();
            tokio::spawn(async move { me.connection(stream).await });
        }
    }

    /// Serves one TCP connection: the WebSocket handshake, `hello`, then frames until it
    /// closes.
    pub async fn connection(&self, stream: TcpStream) {
        let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        let (mut sink, mut source) = ws.split();
        let hello = loop {
            match source.next().await {
                Some(Ok(WsMessage::Text(t))) => break Frame::parse(t.as_str()),
                Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            }
        };
        let Ok(Frame::Hello {
            node,
            credential,
            hosts,
        }) = hello
        else {
            let reason = "the first frame must be hello".to_owned();
            let _ = sink.send(text(&Frame::Refused { reason })).await;
            let _ = sink.close().await;
            return;
        };
        if let Some(check) = &self.inner.options.credentials
            && let Err(reason) = check(credential.as_deref())
        {
            let _ = sink.send(text(&Frame::Refused { reason })).await;
            let _ = sink.close().await;
            return;
        }
        let link = lock(&self.inner.links)
            .entry(node.clone())
            .or_insert_with(|| Arc::new(Link::new()))
            .clone();
        {
            let mut p = lock(&self.inner.prefixes);
            p.retain(|(_, n)| n != &node);
            p.extend(hosts.into_iter().map(|h| (h, node.clone())));
        }
        let welcome = Frame::Welcome {
            node: self.inner.options.node.clone(),
            hosts: self.inner.options.hosts.clone(),
        };
        if sink.send(text(&welcome)).await.is_err() {
            return;
        }
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst);
        let (tx, mut rx) = unbounded::<String>();
        let mut kill = self.inner.kill.subscribe();
        link.connected(generation, tx);
        loop {
            tokio::select! {
                out = rx.next() => {
                    let Some(t) = out else { break };
                    let twice = self.inner.duplicate.load(Ordering::SeqCst)
                        && t.contains("\"type\":\"message\"");
                    if sink.send(WsMessage::Text(t.clone().into())).await.is_err() {
                        break;
                    }
                    if twice && sink.send(WsMessage::Text(t.into())).await.is_err() {
                        break;
                    }
                }
                m = source.next() => match m {
                    Some(Ok(WsMessage::Text(t))) => self.inner.inbound(&link, t.as_str()),
                    Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {}
                },
                _ = kill.changed() => break,
            }
        }
        link.disconnected(generation);
        let _ = sink.close().await;
    }

    /// Closes every open connection (clients reconnect); for maintenance and fault tests.
    pub fn disconnect_all(&self) {
        self.inner.kill.send_modify(|n| *n += 1);
    }

    /// Fault injection for tests: while on, received messages are delivered but not
    /// acknowledged, as if the connection dropped before the acknowledgement went out.
    pub fn set_hold_acks(&self, on: bool) {
        self.inner.hold_acks.store(on, Ordering::SeqCst);
    }

    /// Fault injection for tests: while on, every message frame to a client is sent twice.
    pub fn set_duplicate(&self, on: bool) {
        self.inner.duplicate.store(on, Ordering::SeqCst);
    }

    /// The number of client nodes that ever connected.
    pub fn clients(&self) -> usize {
        lock(&self.inner.links).len()
    }

    /// The number of messages to clients not yet acknowledged.
    pub fn pending(&self) -> usize {
        lock(&self.inner.links)
            .values()
            .map(|l| l.pending_len())
            .sum()
    }

    /// The type IRI of the processor this endpoint serves.
    pub fn processor(&self) -> &'static str {
        WEBSOCKET_PROCESSOR
    }
}

fn text(f: &Frame) -> WsMessage {
    WsMessage::Text(f.to_text().into())
}
