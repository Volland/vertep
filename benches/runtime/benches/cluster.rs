#![allow(missing_docs)] // criterion_group! generates undocumented items
//! Cross-node send round trip: one frame from node n1 to node n2 over loopback QUIC with mutual
//! TLS, until n2's `Ack` is back.

use std::net::SocketAddr;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vertep_cluster::transport::{
    BoxFuture, ClusterCa, ClusterTransport, FrameHandler, NodeIdentity, PeerSet, Receiving,
    TransportConfig,
};
use vertep_cluster::{ClusterId, EpochFilter, Frame, MemoryDeadLetters, NodeId, Reply};

/// Acknowledges every frame without doing anything else.
struct AckAll;

impl FrameHandler for AckAll {
    fn handle(&self, _from: NodeId, frame: Frame) -> BoxFuture<'static, Reply> {
        Box::pin(async move { Reply::Ack { id: frame.id } })
    }
}

fn node(s: &str) -> NodeId {
    NodeId::new(s).expect("node id")
}

fn bind(id: NodeIdentity, peer: &str) -> ClusterTransport {
    let localhost: SocketAddr = "127.0.0.1:0".parse().expect("address");
    ClusterTransport::bind(
        localhost,
        id,
        Arc::new(PeerSet::new([node(peer)])),
        Receiving {
            handler: Arc::new(AckAll),
            epochs: Arc::new(EpochFilter::new()),
            dead_letters: Arc::new(MemoryDeadLetters::default()),
        },
        TransportConfig::default(),
    )
    .expect("transport binds")
}

// @tg: implements:: [[openspec:runtime-performance#Hot Path Benchmarks]]
fn send_round_trip(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let ca = ClusterCa::generate(&ClusterId::new("bench").expect("cluster id")).expect("ca");
    let (a, b) = rt.block_on(async {
        let b = bind(ca.issue(&node("n2")).expect("n2 identity"), "n1");
        let a = bind(ca.issue(&node("n1")).expect("n1 identity"), "n2");
        (a, b)
    });
    let b_addr = b.local_addr().expect("n2 address");
    let n2 = node("n2");
    let frame = Frame {
        id: "urn:vertep:bench:m".into(),
        sender: "urn:vertep:bench:x".into(),
        target: "urn:vertep:bench:y".into(),
        fence: None,
        hops: 0,
        event: "ping".into(),
        data: Vec::new(),
    };
    // Connect and open the pair stream before measuring.
    let first = rt
        .block_on(a.send(&n2, b_addr, &frame))
        .expect("first send");
    assert!(matches!(first, Reply::Ack { .. }), "{first:?}");

    let mut g = c.benchmark_group("cluster");
    g.throughput(Throughput::Elements(1));
    g.bench_function(BenchmarkId::new("send_round_trip", "loopback"), |bench| {
        bench.iter(|| {
            let reply = rt.block_on(a.send(&n2, b_addr, &frame)).expect("send");
            debug_assert!(matches!(reply, Reply::Ack { .. }));
        });
    });
    g.finish();
    drop((a, b));
}

criterion_group!(benches, send_round_trip);
criterion_main!(benches);
