//! Journal shipping over the QUIC cluster transport.
//!
//! A pull travels as a request [`Frame`] with the event [`PULL_EVENT`] and the encoded
//! [`PullRequest`] as its data, on the stream of the pair (`urn:vertep:shipping:<follower>`,
//! actor), so one follower's pulls of one journal are sent one at a time, in order. The leader's
//! [`ShipService`] answers it with [`Reply::Data`] holding the encoded batch or refusal; every
//! other frame goes to the node's own handler. Pulls carry no fence and are never deduplicated:
//! a pull is a read, and repeating it is harmless.

use core::fmt;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use vertep_store::JournalStore;
use vertep_store::oxrdf::NamedNode;

use super::{
    Accepted, CatchUp, PULL_EVENT, PullBatch, PullError, PullRequest, PullSource, ShipFollower,
    decode_reply, encode_reply,
};
use crate::frame::{Frame, RejectKind, Reply};
use crate::meta::NodeId;
use crate::placement::ShardId;
use crate::transport::{BoxFuture, ClusterTransport, FrameHandler};

/// The leader's side on the transport: answers pull frames for the shards it serves and hands
/// every other frame to the node's handler.
// @tg: implements:: [[openspec:journal-shipping#Ordered Shipping]]
pub struct ShipService {
    sources: Mutex<BTreeMap<ShardId, Arc<dyn PullSource>>>,
    inner: Option<Arc<dyn FrameHandler>>,
}

impl fmt::Debug for ShipService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShipService").finish_non_exhaustive()
    }
}

impl ShipService {
    /// A service in front of `inner`, the node's handler of message frames (`None` for a node
    /// that only ships).
    pub fn new(inner: Option<Arc<dyn FrameHandler>>) -> Self {
        ShipService {
            sources: Mutex::new(BTreeMap::new()),
            inner,
        }
    }

    /// Answers pulls of `source`'s shard from now on.
    pub fn serve(&self, source: Arc<dyn PullSource>) {
        self.lock().insert(source.shard(), source);
    }

    /// Stops answering pulls of `shard` (the node handed it over).
    pub fn stop(&self, shard: ShardId) {
        self.lock().remove(&shard);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<ShardId, Arc<dyn PullSource>>> {
        self.sources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn answer(&self, from: &NodeId, data: &[u8]) -> Result<PullBatch, PullError> {
        let req = PullRequest::decode(data)?;
        let source = self
            .lock()
            .get(&req.shard)
            .cloned()
            .ok_or(PullError::NotLeader { shard: req.shard })?;
        source.serve(from, &req)
    }
}

impl FrameHandler for ShipService {
    fn handle(&self, from: NodeId, frame: Frame) -> BoxFuture<'static, Reply> {
        if frame.event == PULL_EVENT {
            let data = encode_reply(&self.answer(&from, &frame.data));
            return Box::pin(async move { Reply::Data { id: frame.id, data } });
        }
        match &self.inner {
            Some(inner) => inner.handle(from, frame),
            None => Box::pin(async move {
                Reply::Rejected {
                    id: frame.id,
                    kind: RejectKind::Unavailable,
                    message: "this node only ships journals".into(),
                }
            }),
        }
    }
}

/// A follower's link to a leader over the cluster transport.
// @tg: implements:: [[openspec:journal-shipping#Resumption]]
pub struct QuicShipLink {
    transport: Arc<ClusterTransport>,
    leader: NodeId,
    addr: Mutex<SocketAddr>,
    counter: AtomicU64,
}

impl fmt::Debug for QuicShipLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuicShipLink")
            .field("leader", &self.leader)
            .finish_non_exhaustive()
    }
}

impl QuicShipLink {
    /// The link from `transport`'s node to `leader` at `addr`.
    pub fn new(transport: Arc<ClusterTransport>, leader: NodeId, addr: SocketAddr) -> Self {
        QuicShipLink {
            transport,
            leader,
            addr: Mutex::new(addr),
            counter: AtomicU64::new(0),
        }
    }

    /// Points the link at the leader's new address (it restarted elsewhere).
    pub fn set_addr(&self, addr: SocketAddr) {
        *self
            .addr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = addr;
    }

    /// Sends one pull and returns the leader's answer.
    ///
    /// # Errors
    ///
    /// [`PullError::Unreachable`] when the connection fails or the leader cannot take frames,
    /// the leader's refusal, [`PullError::Malformed`] for an answer that is not one.
    pub async fn pull(&self, request: &PullRequest) -> Result<PullBatch, PullError> {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let me = self.transport.node();
        let frame = Frame {
            id: format!("pull:{me}:{}:{}:{n}", request.actor, request.after.get()),
            sender: format!("urn:vertep:shipping:{me}"),
            target: request.actor.as_str().to_owned(),
            fence: None,
            hops: 0,
            event: PULL_EVENT.to_owned(),
            data: request.encode(),
        };
        let addr = *self
            .addr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match self.transport.send(&self.leader, addr, &frame).await {
            Ok(Reply::Data { data, .. }) => decode_reply(&data)?,
            Ok(Reply::Rejected { message, .. }) => Err(PullError::Unreachable(message)),
            Ok(other) => Err(PullError::Malformed(format!(
                "a pull answered with {other:?}"
            ))),
            Err(e) => Err(PullError::Unreachable(e.to_string())),
        }
    }

    /// [`ShipFollower::catch_up`] over the transport: pulls until `follower` holds the leader's
    /// head, re-pulling from its durable position after every refusal.
    ///
    /// # Errors
    ///
    /// The first [`PullError`] of the link; calling it again resumes from the durable position.
    pub async fn catch_up<J: JournalStore>(
        &self,
        follower: &ShipFollower<J>,
        actor: &NamedNode,
        max_rounds: usize,
        observe: &mut (dyn FnMut(&PullBatch, &Accepted) + Send),
    ) -> Result<CatchUp, PullError> {
        let mut out = CatchUp::default();
        while out.rounds < max_rounds {
            let req = follower
                .request(actor)
                .map_err(|e| PullError::Store(e.to_string()))?;
            let batch = self.pull(&req).await?;
            let accepted = follower.accept(&batch)?;
            out.rounds += 1;
            out.stored += accepted.stored();
            out.head = accepted.head;
            observe(&batch, &accepted);
            if accepted.refusal().is_some() {
                out.refused += 1;
                continue;
            }
            if batch.entries.is_empty() && accepted.head >= batch.head {
                break;
            }
        }
        Ok(out)
    }
}
