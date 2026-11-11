//! The cluster transport: frames, routing table and redirects, and QUIC with mutual TLS over
//! loopback.
#![cfg(feature = "quic")]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{meta, node};
use proptest::prelude::*;
use tokio::sync::{Notify, mpsc};
use vertep_cluster::frame::{RejectKind, delimit};
use vertep_cluster::transport::{
    BoxFuture, ClusterCa, ClusterTransport, FrameHandler, NodeIdentity, PeerSet, Receiving,
    TransportConfig, TransportError,
};
use vertep_cluster::{
    ClusterId, EpochFilter, Fence, Frame, FrameError, MemoryDeadLetters, NodeId, Redirect, Reply,
    RoutingTable, ShardId, ShardOwner, View,
};

fn view_with(version: u64, owners: &[(u32, &str, u64)]) -> View {
    let mut v = View::empty(ClusterId::new("test").unwrap());
    v.version = version;
    for (s, n, e) in owners {
        v.owners.insert(
            ShardId::new(*s),
            ShardOwner {
                node: node(n),
                epoch: *e,
            },
        );
    }
    v
}

fn actor_of_shard(s: u32) -> String {
    common::actor_in(&meta(64), ShardId::new(s), "route")
        .as_str()
        .to_owned()
}

// @tg: verifies:: [[openspec:cluster-transport#Routing Via The Shard Table#Message routed to the owner]] {level: unit}
#[test]
fn message_routed_to_the_owner() {
    let mut t = RoutingTable::new(meta(64));
    t.apply_view(&view_with(1, &[(3, "c", 4)]));
    let (s, owner) = t.route(&actor_of_shard(3));
    assert_eq!(s, ShardId::new(3));
    assert_eq!(owner.unwrap().node, node("c"));
}

// @tg: verifies:: [[openspec:cluster-transport#Routing Via The Shard Table#Routing follows a handover]] {level: unit}
#[test]
fn routing_follows_a_handover() {
    let mut t = RoutingTable::new(meta(64));
    t.apply_view(&view_with(1, &[(3, "c", 4)]));
    assert!(t.apply_view(&view_with(2, &[(3, "d", 5)])));
    assert_eq!(t.route(&actor_of_shard(3)).1.unwrap().node, node("d"));
    // an older view never moves it back
    assert!(!t.apply_view(&view_with(1, &[(3, "c", 4)])));
    assert_eq!(t.owner(ShardId::new(3)).unwrap().node, node("d"));
}

// @tg: verifies:: [[openspec:cluster-transport#Redirect On Stale Routing#Sender learns the new owner]] {level: unit}
#[test]
fn sender_learns_the_new_owner() {
    let mut t = RoutingTable::new(meta(64));
    t.apply_view(&view_with(1, &[(3, "c", 4)]));
    let r = Redirect {
        shard: ShardId::new(3),
        owner: node("d"),
        epoch: 8,
    };
    assert!(t.apply_redirect(&r));
    let o = t.route(&actor_of_shard(3)).1.unwrap();
    assert_eq!((o.node.clone(), o.epoch), (node("d"), 8));
}

// @tg: verifies:: [[openspec:cluster-transport#Redirect On Stale Routing#Redirect with an older epoch is ignored]] {level: unit}
#[test]
fn redirect_with_an_older_epoch_is_ignored() {
    let mut t = RoutingTable::new(meta(64));
    t.apply_view(&view_with(1, &[(3, "d", 8)]));
    let stale = Redirect {
        shard: ShardId::new(3),
        owner: node("c"),
        epoch: 5,
    };
    assert!(!t.apply_redirect(&stale));
    assert_eq!(t.owner(ShardId::new(3)).unwrap().node, node("d"));
}

fn frame(id: &str, sender: &str, target: &str) -> Frame {
    Frame {
        id: id.into(),
        sender: sender.into(),
        target: target.into(),
        fence: None,
        hops: 0,
        event: "ev".into(),
        data: Vec::new(),
    }
}

fn arb_frame() -> impl Strategy<Value = Frame> {
    (
        ".{0,40}",
        ".{0,60}",
        ".{0,60}",
        proptest::option::of((any::<u32>(), any::<u64>())),
        any::<u8>(),
        ".{0,30}",
        proptest::collection::vec(any::<u8>(), 0..512),
    )
        .prop_map(|(id, sender, target, fence, hops, event, data)| Frame {
            id,
            sender,
            target,
            fence: fence.map(|(s, e)| Fence {
                shard: ShardId::new(s),
                epoch: e,
            }),
            hops,
            event,
            data,
        })
}

proptest! {
    #[test]
    fn frames_round_trip(f in arb_frame()) {
        prop_assert_eq!(Frame::decode(&f.encode()).unwrap(), f);
    }

    #[test]
    fn replies_round_trip(id in ".{0,40}", epoch in any::<u64>(), msg in ".{0,40}") {
        for r in [
            Reply::Ack { id: id.clone() },
            Reply::Forwarded { id: id.clone(), redirect: Redirect { shard: ShardId::new(3), owner: node("d"), epoch } },
            Reply::Rejected { id: id.clone(), kind: RejectKind::Fenced, message: msg.clone() },
        ] {
            prop_assert_eq!(Reply::decode(&r.encode()).unwrap(), r);
        }
    }
}

#[test]
fn delimit_refuses_oversized_payloads() {
    assert!(matches!(
        delimit(&[0u8; 10], 9),
        Err(FrameError::TooLarge { size: 10, max: 9 })
    ));
    assert_eq!(delimit(&[1, 2], 9).unwrap(), vec![0, 0, 0, 2, 1, 2]);
}

/// Bounded mailboxes per target; a delivery waits for room, then acks.
struct Mailboxes {
    cap: usize,
    boxes: Mutex<HashMap<String, mpsc::Sender<(NodeId, Frame)>>>,
    outlets: Mutex<HashMap<String, mpsc::Receiver<(NodeId, Frame)>>>,
    calls: AtomicUsize,
    stalled_pair: Option<(String, String)>,
    release: Notify,
    unavailable: bool,
}

impl Mailboxes {
    fn new(cap: usize) -> Self {
        Mailboxes {
            cap,
            boxes: Mutex::new(HashMap::new()),
            outlets: Mutex::new(HashMap::new()),
            calls: AtomicUsize::new(0),
            stalled_pair: None,
            release: Notify::new(),
            unavailable: false,
        }
    }

    fn sender(&self, target: &str) -> mpsc::Sender<(NodeId, Frame)> {
        let mut boxes = self.boxes.lock().unwrap();
        boxes
            .entry(target.to_owned())
            .or_insert_with(|| {
                let (tx, rx) = mpsc::channel(self.cap);
                self.outlets.lock().unwrap().insert(target.to_owned(), rx);
                tx
            })
            .clone()
    }

    fn outlet(&self, target: &str) -> mpsc::Receiver<(NodeId, Frame)> {
        self.sender(target);
        self.outlets.lock().unwrap().remove(target).unwrap()
    }
}

struct Handler(Arc<Mailboxes>);

impl FrameHandler for Handler {
    fn handle(&self, from: NodeId, frame: Frame) -> BoxFuture<'static, Reply> {
        let me = self.0.clone();
        Box::pin(async move {
            me.calls.fetch_add(1, Ordering::SeqCst);
            if me.unavailable {
                return Reply::Rejected {
                    id: frame.id,
                    kind: RejectKind::Unavailable,
                    message: "mailbox not ready".into(),
                };
            }
            if me.stalled_pair.as_ref() == Some(&(frame.sender.clone(), frame.target.clone())) {
                me.release.notified().await;
            }
            let id = frame.id.clone();
            let tx = me.sender(&frame.target);
            let _ = tx.send((from, frame)).await;
            Reply::Ack { id }
        })
    }
}

struct Pair {
    ca: ClusterCa,
    a: ClusterTransport,
    b_addr: SocketAddr,
    b_policy: Arc<PeerSet>,
    mailboxes: Arc<Mailboxes>,
    dead: Arc<MemoryDeadLetters>,
    _b: ClusterTransport,
}

fn localhost() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

fn transport(
    id: NodeIdentity,
    admitted: Arc<PeerSet>,
    handler: Arc<Mailboxes>,
    dead: Arc<MemoryDeadLetters>,
    config: TransportConfig,
) -> ClusterTransport {
    ClusterTransport::bind(
        localhost(),
        id,
        admitted,
        Receiving {
            handler: Arc::new(Handler(handler)),
            epochs: Arc::new(EpochFilter::new()),
            dead_letters: dead,
        },
        config,
    )
    .unwrap()
}

fn pair_with(mailboxes: Mailboxes, b_config: TransportConfig) -> Pair {
    let cluster = ClusterId::new("test").unwrap();
    let ca = ClusterCa::generate(&cluster).unwrap();
    let mailboxes = Arc::new(mailboxes);
    let dead = Arc::new(MemoryDeadLetters::default());
    let b_policy = Arc::new(PeerSet::new([node("n1")]));
    let b = transport(
        ca.issue(&node("n2")).unwrap(),
        b_policy.clone(),
        mailboxes.clone(),
        dead.clone(),
        b_config,
    );
    let a = transport(
        ca.issue(&node("n1")).unwrap(),
        Arc::new(PeerSet::new([node("n2")])),
        Arc::new(Mailboxes::new(16)),
        Arc::new(MemoryDeadLetters::default()),
        TransportConfig::default(),
    );
    Pair {
        b_addr: b.local_addr().unwrap(),
        ca,
        a,
        b_policy,
        mailboxes,
        dead,
        _b: b,
    }
}

fn pair() -> Pair {
    pair_with(Mailboxes::new(64), TransportConfig::default())
}

async fn within<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(10), f)
        .await
        .expect("timed out")
}

// @tg: verifies:: [[openspec:cluster-transport#Mutual TLS Within The Cluster#Member connects]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn member_connects() {
    let p = pair();
    let mut y = p.mailboxes.outlet("urn:y");
    let reply = within(p.a.send(&node("n2"), p.b_addr, &frame("m1", "urn:x", "urn:y")))
        .await
        .unwrap();
    assert_eq!(reply, Reply::Ack { id: "m1".into() });
    let (from, got) = within(y.recv()).await.unwrap();
    assert_eq!(from, node("n1"));
    assert_eq!(got.id, "m1");
}

// @tg: verifies:: [[openspec:cluster-transport#Mutual TLS Within The Cluster#Foreign certificate refused]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_certificate_refused() {
    let p = pair();
    // same cluster name and node name, but issued by another CA
    let foreign_ca = ClusterCa::generate(&ClusterId::new("test").unwrap()).unwrap();
    let mut foreign = foreign_ca.issue(&node("n1")).unwrap();
    foreign.ca = p.ca.certificate().clone(); // it even trusts the real cluster CA
    let intruder = transport(
        foreign,
        Arc::new(PeerSet::new([node("n2")])),
        Arc::new(Mailboxes::new(1)),
        Arc::new(MemoryDeadLetters::default()),
        TransportConfig::default(),
    );
    let r = within(intruder.send(&node("n2"), p.b_addr, &frame("m1", "urn:x", "urn:y"))).await;
    assert!(
        matches!(
            r,
            Err(TransportError::Connect(_) | TransportError::Stream(_))
        ),
        "{r:?}"
    );
    assert_eq!(
        p.mailboxes.calls.load(Ordering::SeqCst),
        0,
        "no frame was read"
    );
}

// @tg: verifies:: [[openspec:cluster-transport#Mutual TLS Within The Cluster#Removed member refused]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removed_member_refused() {
    let p = pair();
    // n1 left the membership view
    p.b_policy
        .follow(&View::empty(ClusterId::new("test").unwrap()));
    let r = within(p.a.send(&node("n2"), p.b_addr, &frame("m1", "urn:x", "urn:y"))).await;
    assert!(r.is_err(), "{r:?}");
    assert_eq!(p.mailboxes.calls.load(Ordering::SeqCst), 0);
    // readmitted, a new connection succeeds
    p.b_policy.set([node("n1")]);
    let r = within(p.a.send(&node("n2"), p.b_addr, &frame("m2", "urn:x", "urn:y"))).await;
    assert_eq!(r.unwrap(), Reply::Ack { id: "m2".into() });
}

// @tg: verifies:: [[openspec:cluster-transport#Message Framing#Frame decodes to the same event]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frame_decodes_to_the_same_event() {
    let p = pair();
    let mut y = p.mailboxes.outlet("urn:vt:actor:y");
    let sent = Frame {
        id: "01J9Z-message".into(),
        sender: "urn:vt:actor:x".into(),
        target: "urn:vt:actor:y".into(),
        fence: Some(Fence {
            shard: ShardId::new(9),
            epoch: 6,
        }),
        hops: 1,
        event: "order.placed".into(),
        data: r#"{"order":{"id":42,"lines":[{"sku":"a","qty":2}],"note":"ü"}}"#
            .as_bytes()
            .to_vec(),
    };
    within(p.a.send(&node("n2"), p.b_addr, &sent))
        .await
        .unwrap();
    let (_, got) = within(y.recv()).await.unwrap();
    assert_eq!(got, sent);
}

// @tg: verifies:: [[openspec:cluster-transport#Message Framing#Oversized frame rejected]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversized_frame_rejected() {
    let p = pair_with(
        Mailboxes::new(64),
        TransportConfig {
            max_frame: 1024,
            ..TransportConfig::default()
        },
    );
    let mut big = frame("big", "urn:x", "urn:y");
    big.data = vec![7; 4096];
    let reply = within(p.a.send(&node("n2"), p.b_addr, &big)).await.unwrap();
    assert!(
        matches!(
            &reply,
            Reply::Rejected {
                kind: RejectKind::TooLarge,
                ..
            }
        ),
        "{reply:?}"
    );
    // the connection and even the stream stay open for other frames
    let reply = within(p.a.send(&node("n2"), p.b_addr, &frame("small", "urn:x", "urn:y")))
        .await
        .unwrap();
    assert_eq!(reply, Reply::Ack { id: "small".into() });
    assert_eq!(p.mailboxes.calls.load(Ordering::SeqCst), 1);
    // a sender never puts a frame over its own maximum on the wire
    big.data = vec![0; 2 << 20];
    assert!(matches!(
        within(p.a.send(&node("n2"), p.b_addr, &big)).await,
        Err(TransportError::Frame(FrameError::TooLarge { .. }))
    ));
}

// @tg: verifies:: [[openspec:cluster-transport#Ordering Per Sender And Receiver#Two sends arrive in order]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_sends_arrive_in_order() {
    let p = pair();
    let mut y = p.mailboxes.outlet("urn:y");
    for i in 0..50 {
        within(p.a.send(
            &node("n2"),
            p.b_addr,
            &frame(&format!("e{i}"), "urn:x", "urn:y"),
        ))
        .await
        .unwrap();
    }
    for i in 0..50 {
        let (_, f) = within(y.recv()).await.unwrap();
        assert_eq!(f.id, format!("e{i}"));
    }
}

// @tg: verifies:: [[openspec:cluster-transport#Ordering Per Sender And Receiver#Independent pairs do not block each other]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_pairs_do_not_block_each_other() {
    let mut m = Mailboxes::new(64);
    m.stalled_pair = Some(("urn:x".into(), "urn:y".into()));
    let p = Arc::new(pair_with(m, TransportConfig::default()));
    let stalled = {
        let p = p.clone();
        tokio::spawn(async move {
            p.a.send(&node("n2"), p.b_addr, &frame("x-y", "urn:x", "urn:y"))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!stalled.is_finished(), "delivery from x to y is stalled");
    for (id, s, t) in [("x-z", "urn:x", "urn:z"), ("w-y", "urn:w", "urn:y")] {
        let r = within(p.a.send(&node("n2"), p.b_addr, &frame(id, s, t)))
            .await
            .unwrap();
        assert_eq!(r, Reply::Ack { id: id.into() });
    }
    p.mailboxes.release.notify_one();
    assert_eq!(
        within(stalled).await.unwrap().unwrap(),
        Reply::Ack { id: "x-y".into() }
    );
}

// @tg: verifies:: [[openspec:cluster-transport#Flow Control And Backpressure#Full mailbox slows the sender]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_mailbox_slows_the_sender() {
    let p = Arc::new(pair_with(Mailboxes::new(2), TransportConfig::default()));
    let mut y = p.mailboxes.outlet("urn:y");
    let acked = Arc::new(AtomicUsize::new(0));
    let dispatch = {
        let (p, acked) = (p.clone(), acked.clone());
        tokio::spawn(async move {
            for i in 0..3 {
                p.a.send(
                    &node("n2"),
                    p.b_addr,
                    &frame(&format!("e{i}"), "urn:x", "urn:y"),
                )
                .await
                .unwrap();
                acked.fetch_add(1, Ordering::SeqCst);
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        acked.load(Ordering::SeqCst),
        2,
        "the third send waits for room"
    );
    let mut got = vec![within(y.recv()).await.unwrap().1.id];
    within(dispatch).await.unwrap();
    assert_eq!(acked.load(Ordering::SeqCst), 3);
    got.push(within(y.recv()).await.unwrap().1.id);
    got.push(within(y.recv()).await.unwrap().1.id);
    assert_eq!(got, ["e0", "e1", "e2"], "nothing dropped");
}

// @tg: verifies:: [[openspec:cluster-transport#Flow Control And Backpressure#Backpressure is per mailbox]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backpressure_is_per_mailbox() {
    let p = Arc::new(pair_with(Mailboxes::new(1), TransportConfig::default()));
    let _y = p.mailboxes.outlet("urn:y");
    let mut z = p.mailboxes.outlet("urn:z");
    within(p.a.send(&node("n2"), p.b_addr, &frame("y1", "urn:x", "urn:y")))
        .await
        .unwrap();
    let blocked = {
        let p = p.clone();
        tokio::spawn(async move {
            p.a.send(&node("n2"), p.b_addr, &frame("y2", "urn:x", "urn:y"))
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!blocked.is_finished(), "y's mailbox is full");
    let r = within(p.a.send(&node("n2"), p.b_addr, &frame("z1", "urn:x", "urn:z")))
        .await
        .unwrap();
    assert_eq!(r, Reply::Ack { id: "z1".into() });
    assert_eq!(within(z.recv()).await.unwrap().1.id, "z1");
    blocked.abort();
}

// @tg: verifies:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication#Lost acknowledgement causes a duplicate that is discarded]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_acknowledgement_causes_a_duplicate_that_is_discarded() {
    let p = pair();
    let mut y = p.mailboxes.outlet("urn:y");
    let m = frame("m1", "urn:x", "urn:y");
    within(p.a.send(&node("n2"), p.b_addr, &m)).await.unwrap();
    // the ack was lost: the sender retries the same message
    let retry = within(p.a.send(&node("n2"), p.b_addr, &m)).await.unwrap();
    assert_eq!(retry, Reply::Ack { id: "m1".into() });
    assert_eq!(
        p.mailboxes.calls.load(Ordering::SeqCst),
        1,
        "delivered once"
    );
    assert_eq!(within(y.recv()).await.unwrap().1.id, "m1");
    assert!(y.try_recv().is_err());
}

// @tg: verifies:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication#Retries stop at the deadline]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retries_stop_at_the_deadline() {
    let mut m = Mailboxes::new(1);
    m.unavailable = true;
    let p = pair_with(m, TransportConfig::default());
    let started = std::time::Instant::now();
    let r = within(p.a.send_reliable(
        &node("n2"),
        p.b_addr,
        &frame("m1", "urn:x", "urn:y"),
        Duration::from_millis(300),
        Duration::from_millis(50),
    ))
    .await;
    assert!(
        matches!(r, Err(TransportError::Deadline { ref id, .. }) if id == "m1"),
        "{r:?}"
    );
    assert!(p.mailboxes.calls.load(Ordering::SeqCst) >= 2, "it retried");
    assert!(started.elapsed() < Duration::from_secs(2));
}

// @tg: verifies:: [[openspec:shard-placement#Journal Fencing Backstop#Fenced outbound messages are dropped]] {level: integration}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fenced_outbound_messages_are_dropped() {
    let p = pair();
    let mut y = p.mailboxes.outlet("urn:y");
    let mut new_owner = frame("from-b", "urn:x", "urn:y");
    new_owner.fence = Some(Fence {
        shard: ShardId::new(9),
        epoch: 6,
    });
    let mut stale = frame("from-a", "urn:x2", "urn:y");
    stale.fence = Some(Fence {
        shard: ShardId::new(9),
        epoch: 5,
    });
    assert_eq!(
        within(p.a.send(&node("n2"), p.b_addr, &new_owner))
            .await
            .unwrap(),
        Reply::Ack {
            id: "from-b".into()
        }
    );
    let r = within(p.a.send(&node("n2"), p.b_addr, &stale))
        .await
        .unwrap();
    assert!(
        matches!(
            r,
            Reply::Rejected {
                kind: RejectKind::Fenced,
                ..
            }
        ),
        "{r:?}"
    );
    let dead = p.dead.fenced_messages();
    assert_eq!(dead.len(), 1);
    assert_eq!((dead[0].id.as_str(), dead[0].highest), ("from-a", 6));
    assert_eq!(within(y.recv()).await.unwrap().1.id, "from-b");
    assert!(y.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairs_beyond_the_stream_cap_share_streams_in_order() {
    let cluster = ClusterId::new("test").unwrap();
    let ca = ClusterCa::generate(&cluster).unwrap();
    let mailboxes = Arc::new(Mailboxes::new(256));
    let dead = Arc::new(MemoryDeadLetters::default());
    let b = transport(
        ca.issue(&node("n2")).unwrap(),
        Arc::new(PeerSet::new([node("n1")])),
        mailboxes.clone(),
        dead,
        TransportConfig::default(),
    );
    let a = transport(
        ca.issue(&node("n1")).unwrap(),
        Arc::new(PeerSet::new([node("n2")])),
        Arc::new(Mailboxes::new(1)),
        Arc::new(MemoryDeadLetters::default()),
        TransportConfig {
            max_pair_streams: 2,
            ..TransportConfig::default()
        },
    );
    let addr = b.local_addr().unwrap();
    let mut outlets: Vec<_> = (0..5)
        .map(|t| mailboxes.outlet(&format!("urn:t{t}")))
        .collect();
    for i in 0..10 {
        for t in 0..5 {
            let f = frame(&format!("{t}-{i}"), "urn:x", &format!("urn:t{t}"));
            within(a.send(&node("n2"), addr, &f)).await.unwrap();
        }
    }
    for (t, o) in outlets.iter_mut().enumerate() {
        for i in 0..10 {
            assert_eq!(within(o.recv()).await.unwrap().1.id, format!("{t}-{i}"));
        }
    }
}
