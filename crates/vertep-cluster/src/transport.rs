//! The QUIC cluster transport: mutual TLS with cluster-issued certificates, one connection per
//! peer, one stream per sender and receiver pair, length-delimited [`Frame`]s answered by a
//! [`Reply`] on the same stream.
//!
//! - **Authentication.** Every node has a certificate issued by the cluster's CA for the DNS
//!   name `<node>.<cluster>.vertep.internal`. A server accepts a client only if its certificate
//!   chains to the cluster CA *and* names a node the [`PeerPolicy`] admits (a current member),
//!   so a foreign certificate and a removed member are both refused during the handshake,
//!   before any frame is read.
//! - **Ordering.** Frames of one `(sender, target)` pair travel on one stream, one at a time,
//!   so they arrive in order; other pairs use other streams and never wait for it. Past
//!   `max_pair_streams` streams per peer, pairs share streams by hash, still in order per pair.
//! - **Backpressure.** The receiver reads the next frame of a stream only after the handler
//!   accepted the previous one (stored it in the mailbox), so a full mailbox slows exactly the
//!   streams that feed it.
//! - **Fencing and deduplication.** A frame whose `(shard, epoch)` is below the highest epoch
//!   seen for the shard is rejected and recorded as a dead letter; a frame whose identifier
//!   was already accepted for its target is acknowledged without being delivered again.
//!
//! The runtime integration (`crate::runtime`) sends runtime envelopes over it through
//! `QuicWire` and receives them with `NodeFrameHandler`; the cluster Event I/O Processor is
//! registered with the runtime there.

use core::fmt;
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, RootCertStore, SignatureScheme};

use crate::fencing::{DeadLetters, EpochFilter};
use crate::frame::{DEFAULT_MAX_FRAME, Frame, FrameError, RejectKind, Reply, delimit};
use crate::meta::{ClusterId, NodeId};
use crate::routing::DedupCache;

/// The ALPN protocol of the cluster transport.
pub const ALPN: &[u8] = b"vertep-cluster/1";

/// A transport error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportError {
    /// Certificates or TLS configuration are invalid.
    Tls(String),
    /// The endpoint could not be bound.
    Io(String),
    /// The connection could not be established (including a refused handshake).
    Connect(String),
    /// The stream failed while sending or awaiting the reply.
    Stream(String),
    /// The frame could not be encoded or the reply decoded.
    Frame(FrameError),
    /// No acknowledgement arrived before the delivery deadline.
    Deadline {
        /// The message identifier.
        id: String,
        /// The last error seen.
        last: String,
    },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Tls(m) => write!(f, "cluster TLS: {m}"),
            TransportError::Io(m) => write!(f, "cluster endpoint: {m}"),
            TransportError::Connect(m) => write!(f, "cluster connection: {m}"),
            TransportError::Stream(m) => write!(f, "cluster stream: {m}"),
            TransportError::Frame(e) => e.fmt(f),
            TransportError::Deadline { id, last } => {
                write!(
                    f,
                    "message {id} not acknowledged before its deadline: {last}"
                )
            }
        }
    }
}

impl std::error::Error for TransportError {}

impl From<FrameError> for TransportError {
    fn from(e: FrameError) -> Self {
        TransportError::Frame(e)
    }
}

fn tls(e: impl fmt::Display) -> TransportError {
    TransportError::Tls(e.to_string())
}

/// The certificate name of `node` in `cluster`: `<node>.<cluster>.vertep.internal`.
pub fn node_name(cluster: &ClusterId, node: &NodeId) -> String {
    format!("{node}.{cluster}.vertep.internal")
}

/// A node's credentials: its certificate chain and key, issued for the cluster, and the
/// cluster CA certificate it trusts.
#[derive(Debug)]
pub struct NodeIdentity {
    /// The cluster.
    pub cluster: ClusterId,
    /// The node.
    pub node: NodeId,
    /// The node's certificate chain, leaf first.
    pub chain: Vec<CertificateDer<'static>>,
    /// The node's private key.
    pub key: PrivateKeyDer<'static>,
    /// The cluster CA certificate.
    pub ca: CertificateDer<'static>,
}

impl Clone for NodeIdentity {
    fn clone(&self) -> Self {
        NodeIdentity {
            cluster: self.cluster.clone(),
            node: self.node.clone(),
            chain: self.chain.clone(),
            key: self.key.clone_key(),
            ca: self.ca.clone(),
        }
    }
}

/// A cluster certificate authority that issues node certificates.
pub struct ClusterCa {
    cluster: ClusterId,
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
    cert: CertificateDer<'static>,
}

impl fmt::Debug for ClusterCa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterCa")
            .field("cluster", &self.cluster)
            .finish_non_exhaustive()
    }
}

impl ClusterCa {
    /// A new self-signed CA for `cluster`.
    ///
    /// # Errors
    ///
    /// [`TransportError::Tls`] when key generation or signing fails.
    pub fn generate(cluster: &ClusterId) -> Result<Self, TransportError> {
        let key = rcgen::KeyPair::generate().map_err(tls)?;
        let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).map_err(tls)?;
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            format!("vertep cluster {cluster} CA"),
        );
        params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];
        let cert = params.self_signed(&key).map_err(tls)?;
        Ok(ClusterCa {
            cluster: cluster.clone(),
            issuer: rcgen::Issuer::new(params, key),
            cert: cert.der().clone(),
        })
    }

    /// The CA certificate.
    pub fn certificate(&self) -> &CertificateDer<'static> {
        &self.cert
    }

    /// Issues the identity of `node`: a certificate for its cluster name, usable as client
    /// and server.
    ///
    /// # Errors
    ///
    /// [`TransportError::Tls`] when key generation or signing fails.
    pub fn issue(&self, node: &NodeId) -> Result<NodeIdentity, TransportError> {
        let key = rcgen::KeyPair::generate().map_err(tls)?;
        let mut params =
            rcgen::CertificateParams::new(vec![node_name(&self.cluster, node)]).map_err(tls)?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, node.as_str());
        params.extended_key_usages = vec![
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
        ];
        let cert = params.signed_by(&key, &self.issuer).map_err(tls)?;
        Ok(NodeIdentity {
            cluster: self.cluster.clone(),
            node: node.clone(),
            chain: vec![cert.der().clone()],
            key: PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            ca: self.cert.clone(),
        })
    }
}

/// Which peers may connect: the current members of the cluster.
pub trait PeerPolicy: Send + Sync + fmt::Debug {
    /// The nodes that may connect now.
    fn admitted(&self) -> Vec<NodeId>;
}

/// A peer policy over an explicit set, updated by the owner (for example from membership
/// views: [`crate::View::is_live`]).
#[derive(Debug, Default)]
pub struct PeerSet(Mutex<Vec<NodeId>>);

impl PeerSet {
    /// The policy admitting `nodes`.
    pub fn new(nodes: impl IntoIterator<Item = NodeId>) -> Self {
        PeerSet(Mutex::new(nodes.into_iter().collect()))
    }

    /// Replaces the admitted set.
    pub fn set(&self, nodes: impl IntoIterator<Item = NodeId>) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = nodes.into_iter().collect();
    }

    /// Admits the live members of `view`.
    pub fn follow(&self, view: &crate::View) {
        self.set(
            view.members
                .keys()
                .filter(|n| view.is_live(n))
                .cloned()
                .collect::<Vec<_>>(),
        );
    }
}

impl PeerPolicy for PeerSet {
    fn admitted(&self) -> Vec<NodeId> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

/// The admitted node `cert` was issued to, if any.
fn identify(cert: &CertificateDer<'_>, cluster: &ClusterId, admitted: &[NodeId]) -> Option<NodeId> {
    let ee = webpki::EndEntityCert::try_from(cert).ok()?;
    admitted.iter().find_map(|n| {
        let name = ServerName::try_from(node_name(cluster, n)).ok()?;
        ee.verify_is_valid_for_subject_name(&name)
            .ok()
            .map(|()| n.clone())
    })
}

/// A client certificate verifier that accepts only certificates chaining to the cluster CA
/// and naming a node the policy admits.
#[derive(Debug)]
struct MemberVerifier {
    inner: Arc<dyn ClientCertVerifier>,
    cluster: ClusterId,
    policy: Arc<dyn PeerPolicy>,
}

impl ClientCertVerifier for MemberVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.inner.root_hint_subjects()
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        self.inner
            .verify_client_cert(end_entity, intermediates, now)?;
        match identify(end_entity, &self.cluster, &self.policy.admitted()) {
            Some(_) => Ok(ClientCertVerified::assertion()),
            None => Err(rustls::Error::General(format!(
                "the peer is not a current member of cluster {}",
                self.cluster
            ))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// A boxed future, for the handler.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The receiving side of the runtime: delivers a frame to its target's mailbox and answers
/// when it is stored (an ack), forwarded, or refused. The transport reads the next frame of
/// the same stream only after the future completes, which is the backpressure.
pub trait FrameHandler: Send + Sync + 'static {
    /// Delivers `frame`, received from `from`.
    fn handle(&self, from: NodeId, frame: Frame) -> BoxFuture<'static, Reply>;
}

/// The transport's limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportConfig {
    /// The largest frame payload accepted or sent.
    pub max_frame: usize,
    /// The most streams opened to one peer before pairs share streams.
    pub max_pair_streams: usize,
    /// How many accepted message identifiers the receiver remembers for deduplication.
    pub dedup_capacity: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        TransportConfig {
            max_frame: DEFAULT_MAX_FRAME,
            max_pair_streams: 1024,
            dedup_capacity: 65_536,
        }
    }
}

/// What the receiving side does with frames: the handler, the epoch filter and the dead-letter
/// sink for fenced frames.
#[derive(Clone)]
pub struct Receiving {
    /// Delivers accepted frames.
    pub handler: Arc<dyn FrameHandler>,
    /// The highest epoch seen per shard.
    pub epochs: Arc<EpochFilter>,
    /// Where fenced frames are recorded.
    pub dead_letters: Arc<dyn DeadLetters>,
}

impl fmt::Debug for Receiving {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiving").finish_non_exhaustive()
    }
}

type PairStream = Arc<tokio::sync::Mutex<(quinn::SendStream, quinn::RecvStream)>>;

#[derive(Default)]
struct PeerConn {
    conn: Option<quinn::Connection>,
    pairs: HashMap<(String, String), PairStream>,
    streams: Vec<PairStream>,
}

/// A node's QUIC endpoint: accepts members' connections and sends frames to peers.
// @tg: implements:: [[openspec:cluster-transport#QUIC Cluster Transport]]
// @tg: implements:: [[openspec:cluster-transport#Mutual TLS Within The Cluster]]
// @tg: implements:: [[openspec:cluster-transport#Ordering Per Sender And Receiver]]
// @tg: implements:: [[openspec:cluster-transport#Flow Control And Backpressure]]
// @tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]
pub struct ClusterTransport {
    endpoint: quinn::Endpoint,
    identity: NodeIdentity,
    client: quinn::ClientConfig,
    config: TransportConfig,
    peers: tokio::sync::Mutex<HashMap<NodeId, PeerConn>>,
    accept: tokio::task::JoinHandle<()>,
}

impl fmt::Debug for ClusterTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterTransport")
            .field("node", &self.identity.node)
            .finish_non_exhaustive()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn roots(identity: &NodeIdentity) -> Result<Arc<RootCertStore>, TransportError> {
    let mut roots = RootCertStore::empty();
    roots.add(identity.ca.clone()).map_err(tls)?;
    Ok(Arc::new(roots))
}

fn client_config(identity: &NodeIdentity) -> Result<quinn::ClientConfig, TransportError> {
    let mut c = rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls)?
        .with_root_certificates(roots(identity)?)
        .with_client_auth_cert(identity.chain.clone(), identity.key.clone_key())
        .map_err(tls)?;
    c.alpn_protocols = vec![ALPN.to_vec()];
    let q = QuicClientConfig::try_from(c).map_err(tls)?;
    Ok(quinn::ClientConfig::new(Arc::new(q)))
}

fn server_config(
    identity: &NodeIdentity,
    policy: Arc<dyn PeerPolicy>,
) -> Result<quinn::ServerConfig, TransportError> {
    let inner = WebPkiClientVerifier::builder_with_provider(roots(identity)?, provider())
        .build()
        .map_err(tls)?;
    let verifier = Arc::new(MemberVerifier {
        inner,
        cluster: identity.cluster.clone(),
        policy,
    });
    let mut s = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(tls)?
        .with_client_cert_verifier(verifier)
        .with_single_cert(identity.chain.clone(), identity.key.clone_key())
        .map_err(tls)?;
    s.alpn_protocols = vec![ALPN.to_vec()];
    let q = QuicServerConfig::try_from(s).map_err(tls)?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(q)))
}

async fn read_len(recv: &mut quinn::RecvStream) -> Result<Option<usize>, String> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => Ok(Some(u32::from_be_bytes(len) as usize)),
        Err(quinn::ReadExactError::FinishedEarly(0)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

async fn skip(recv: &mut quinn::RecvStream, mut n: usize) -> Result<(), String> {
    let mut buf = vec![0u8; 16 * 1024];
    while n > 0 {
        let k = n.min(buf.len());
        recv.read_exact(&mut buf[..k])
            .await
            .map_err(|e| e.to_string())?;
        n -= k;
    }
    Ok(())
}

async fn write_reply(send: &mut quinn::SendStream, reply: &Reply) -> Result<(), String> {
    let bytes = delimit(&reply.encode(), usize::MAX).map_err(|e| e.to_string())?;
    send.write_all(&bytes).await.map_err(|e| e.to_string())
}

struct Inbox {
    receiving: Receiving,
    dedup: Mutex<DedupCache>,
    max_frame: usize,
}

impl Inbox {
    async fn serve_stream(
        self: Arc<Self>,
        from: NodeId,
        mut send: quinn::SendStream,
        mut recv: quinn::RecvStream,
    ) {
        loop {
            let len = match read_len(&mut recv).await {
                Ok(Some(n)) => n,
                Ok(None) | Err(_) => return,
            };
            let reply = if len > self.max_frame {
                if skip(&mut recv, len).await.is_err() {
                    return;
                }
                Reply::Rejected {
                    id: String::new(),
                    kind: RejectKind::TooLarge,
                    message: format!(
                        "frame of {len} bytes exceeds the maximum of {}",
                        self.max_frame
                    ),
                }
            } else {
                let mut payload = vec![0u8; len];
                if recv.read_exact(&mut payload).await.is_err() {
                    return;
                }
                match Frame::decode(&payload) {
                    Ok(frame) => self.deliver(&from, frame).await,
                    Err(e) => Reply::Rejected {
                        id: String::new(),
                        kind: RejectKind::Malformed,
                        message: e.to_string(),
                    },
                }
            };
            if write_reply(&mut send, &reply).await.is_err() {
                return;
            }
        }
    }

    async fn deliver(&self, from: &NodeId, frame: Frame) -> Reply {
        let seen = self
            .dedup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&frame.target, &frame.id);
        if seen {
            return Reply::Ack { id: frame.id };
        }
        if let Some(fence) = frame.fence {
            let admitted = self.receiving.epochs.admit_or_dead_letter(
                &frame.id,
                &frame.sender,
                &frame.target,
                &frame.event,
                fence,
                self.receiving.dead_letters.as_ref(),
            );
            if !admitted {
                return Reply::Rejected {
                    id: frame.id,
                    kind: RejectKind::Fenced,
                    message: format!("epoch {} of {} is superseded", fence.epoch, fence.shard),
                };
            }
        }
        let (target, id) = (frame.target.clone(), frame.id.clone());
        let reply = self.receiving.handler.handle(from.clone(), frame).await;
        if matches!(reply, Reply::Ack { .. } | Reply::Forwarded { .. }) {
            self.dedup
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .first_time(&target, &id);
        }
        reply
    }
}

async fn serve_connection(
    incoming: quinn::Incoming,
    cluster: ClusterId,
    policy: Arc<dyn PeerPolicy>,
    inbox: Arc<Inbox>,
) {
    let Ok(conn) = incoming.await else {
        return; // handshake refused: foreign or removed peer
    };
    let peer = conn
        .peer_identity()
        .and_then(|any| any.downcast::<Vec<CertificateDer<'static>>>().ok())
        .and_then(|certs| {
            certs
                .first()
                .and_then(|c| identify(c, &cluster, &policy.admitted()))
        });
    let Some(peer) = peer else {
        conn.close(1u32.into(), b"not a current member");
        return;
    };
    while let Ok((send, recv)) = conn.accept_bi().await {
        tokio::spawn(inbox.clone().serve_stream(peer.clone(), send, recv));
    }
}

impl ClusterTransport {
    /// Binds the endpoint of `identity` at `addr` and starts accepting members' connections.
    /// Must be called inside a tokio runtime.
    ///
    /// # Errors
    ///
    /// [`TransportError::Tls`] for unusable credentials, [`TransportError::Io`] when the
    /// address cannot be bound.
    pub fn bind(
        addr: SocketAddr,
        identity: NodeIdentity,
        policy: Arc<dyn PeerPolicy>,
        receiving: Receiving,
        config: TransportConfig,
    ) -> Result<Self, TransportError> {
        let server = server_config(&identity, policy.clone())?;
        let client = client_config(&identity)?;
        let endpoint =
            quinn::Endpoint::server(server, addr).map_err(|e| TransportError::Io(e.to_string()))?;
        let inbox = Arc::new(Inbox {
            receiving,
            dedup: Mutex::new(DedupCache::new(config.dedup_capacity)),
            max_frame: config.max_frame,
        });
        let accept_ep = endpoint.clone();
        let cluster = identity.cluster.clone();
        let accept = tokio::spawn(async move {
            while let Some(incoming) = accept_ep.accept().await {
                tokio::spawn(serve_connection(
                    incoming,
                    cluster.clone(),
                    policy.clone(),
                    inbox.clone(),
                ));
            }
        });
        Ok(ClusterTransport {
            endpoint,
            identity,
            client,
            config,
            peers: tokio::sync::Mutex::new(HashMap::new()),
            accept,
        })
    }

    /// The bound address.
    ///
    /// # Errors
    ///
    /// [`TransportError::Io`] when the socket has no address.
    pub fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        self.endpoint
            .local_addr()
            .map_err(|e| TransportError::Io(e.to_string()))
    }

    /// This node.
    pub fn node(&self) -> &NodeId {
        &self.identity.node
    }

    async fn stream(
        &self,
        peer: &NodeId,
        addr: SocketAddr,
        pair: (String, String),
    ) -> Result<PairStream, TransportError> {
        let mut peers = self.peers.lock().await;
        let pc = peers.entry(peer.clone()).or_default();
        if pc.conn.as_ref().is_some_and(|c| c.close_reason().is_some()) {
            *pc = PeerConn::default();
        }
        if let Some(s) = pc.pairs.get(&pair) {
            return Ok(s.clone());
        }
        let conn = match &pc.conn {
            Some(c) => c.clone(),
            None => {
                let name = node_name(&self.identity.cluster, peer);
                let c = self
                    .endpoint
                    .connect_with(self.client.clone(), addr, &name)
                    .map_err(|e| TransportError::Connect(e.to_string()))?
                    .await
                    .map_err(|e| TransportError::Connect(e.to_string()))?;
                pc.conn = Some(c.clone());
                c
            }
        };
        let stream = if pc.streams.len() < self.config.max_pair_streams.max(1) {
            let (send, recv) = conn
                .open_bi()
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;
            let s = Arc::new(tokio::sync::Mutex::new((send, recv)));
            pc.streams.push(s.clone());
            s
        } else {
            let h = crate::meta::HashVersion::Fnv1a64
                .hash64(format!("{}\n{}", pair.0, pair.1).as_bytes());
            pc.streams[(h % pc.streams.len() as u64) as usize].clone()
        };
        pc.pairs.insert(pair, stream.clone());
        Ok(stream)
    }

    async fn drop_peer(&self, peer: &NodeId) {
        self.peers.lock().await.remove(peer);
    }

    /// Sends `frame` to `peer` at `addr` on the stream of its `(sender, target)` pair and
    /// returns the receiver's reply. Frames of one pair are sent one at a time, in call order.
    ///
    /// # Errors
    ///
    /// [`TransportError::Frame`] for a frame over the maximum size (nothing is sent),
    /// [`TransportError::Connect`] when the peer cannot be reached or refuses this node,
    /// [`TransportError::Stream`] when the stream breaks (the connection is dropped and the
    /// next send reconnects).
    // @tg: implements:: [[openspec:cluster-transport#Message Framing]]
    pub async fn send(
        &self,
        peer: &NodeId,
        addr: SocketAddr,
        frame: &Frame,
    ) -> Result<Reply, TransportError> {
        let bytes = delimit(&frame.encode(), self.config.max_frame)?;
        let pair = (frame.sender.clone(), frame.target.clone());
        let stream = self.stream(peer, addr, pair).await?;
        let mut guard = stream.lock().await;
        let (send, recv) = &mut *guard;
        let result = async {
            send.write_all(&bytes).await.map_err(|e| e.to_string())?;
            let len = read_len(recv)
                .await?
                .ok_or_else(|| "the peer closed the stream".to_owned())?;
            let mut payload = vec![0u8; len];
            recv.read_exact(&mut payload)
                .await
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(payload)
        }
        .await;
        drop(guard);
        match result {
            Ok(payload) => Ok(Reply::decode(&payload)?),
            Err(e) => {
                self.drop_peer(peer).await;
                Err(TransportError::Stream(e))
            }
        }
    }

    /// Sends `frame` until the receiver acknowledges it (stored or forwarded) or rejects it
    /// for good, retrying connection failures and [`RejectKind::Unavailable`] every `backoff`
    /// until `deadline` has passed.
    ///
    /// # Errors
    ///
    /// [`TransportError::Deadline`] when no final reply arrived in time; a frame over the
    /// maximum size fails at once.
    // @tg: implements:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication]] {confidence: partial}
    pub async fn send_reliable(
        &self,
        peer: &NodeId,
        addr: SocketAddr,
        frame: &Frame,
        deadline: Duration,
        backoff: Duration,
    ) -> Result<Reply, TransportError> {
        let start = tokio::time::Instant::now();
        loop {
            let last = match self.send(peer, addr, frame).await {
                Ok(Reply::Rejected {
                    kind: RejectKind::Unavailable,
                    message,
                    ..
                }) => message,
                Ok(reply) => return Ok(reply),
                Err(e @ TransportError::Frame(_)) => return Err(e),
                Err(e) => e.to_string(),
            };
            if start.elapsed() + backoff > deadline {
                return Err(TransportError::Deadline {
                    id: frame.id.clone(),
                    last,
                });
            }
            tokio::time::sleep(backoff).await;
        }
    }

    /// Closes every connection and stops accepting.
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"closing");
        self.accept.abort();
    }
}

impl Drop for ClusterTransport {
    fn drop(&mut self) {
        self.close();
    }
}
