//! Shared helpers of the integration tests.
#![allow(dead_code)]

pub mod containers;
pub mod oracle;

use std::sync::Arc;
use std::time::Duration;

use vertep_cluster::testkit::Harness;
use vertep_cluster::{
    Authority, AuthorityConfig, AuthorityMembership, ClusterId, ClusterMeta, DetectorConfig, Drain,
    LeaseConfig, ManualClock, Membership, MemoryAuthority, NodeId, ShardId, StaticConfig,
};
use vertep_store::oxrdf::NamedNode;

pub fn meta(shards: u64) -> ClusterMeta {
    ClusterMeta::new(ClusterId::new("test").unwrap(), shards).unwrap()
}

pub fn node(s: &str) -> NodeId {
    NodeId::new(s).unwrap()
}

/// An actor IRI whose shard under `meta` is `shard`.
pub fn actor_in(meta: &ClusterMeta, shard: ShardId, tag: &str) -> NamedNode {
    (0..)
        .map(|i| format!("urn:vt:actor:{tag}-{i}"))
        .find(|iri| meta.shard_of(iri) == shard)
        .map(NamedNode::new_unchecked)
        .unwrap()
}

/// Lease 10 s with a 2 s margin; suspect after 3 s, failed after 10 s.
pub fn timing() -> AuthorityConfig {
    AuthorityConfig {
        lease: LeaseConfig {
            duration: Duration::from_secs(10),
            safety_margin: Duration::from_secs(2),
        },
        detector: DetectorConfig {
            suspicion_time: Duration::from_secs(3),
            failure_time: Duration::from_secs(10),
        },
    }
}

pub type Node = AuthorityMembership<Arc<MemoryAuthority>>;

/// Nodes on one authority, all on one manual clock.
pub struct AuthorityHarness {
    pub clock: ManualClock,
    pub authority: Arc<MemoryAuthority>,
    pub nodes: Vec<Arc<Node>>,
}

impl AuthorityHarness {
    /// A static cluster of `n` listed nodes.
    pub fn static_cluster(n: usize, shards: u64) -> Self {
        let clock = ManualClock::new();
        let config = StaticConfig {
            meta: meta(shards),
            members: (0..n)
                .map(|i| (node(&format!("n{i}")), format!("10.0.0.{i}:7400")))
                .collect(),
            timing: timing(),
        };
        let authority = Arc::new(config.authority(Arc::new(clock.clone())));
        let nodes: Vec<Arc<Node>> = config
            .members
            .iter()
            .map(|(id, _)| {
                Arc::new(
                    config
                        .membership(id, authority.clone(), Arc::new(clock.clone()))
                        .unwrap(),
                )
            })
            .collect();
        for n in &nodes {
            n.poll().unwrap();
        }
        AuthorityHarness {
            clock,
            authority,
            nodes,
        }
    }

    /// An external cluster (in-memory stand-in) of `n` nodes that joined.
    pub fn external_cluster(n: usize, shards: u64) -> Self {
        let clock = ManualClock::new();
        let authority = Arc::new(MemoryAuthority::new(
            meta(shards),
            timing(),
            Arc::new(clock.clone()),
        ));
        let mut h = AuthorityHarness {
            clock,
            authority,
            nodes: Vec::new(),
        };
        for i in 0..n {
            h.add_node(&format!("n{i}")).join().unwrap();
        }
        h.tick_all();
        h
    }

    pub fn add_node(&mut self, id: &str) -> Arc<Node> {
        let n = Arc::new(AuthorityMembership::new(
            self.authority.clone(),
            self.authority.meta().clone(),
            node(id),
            format!("{id}.local:7400"),
            Arc::new(self.clock.clone()),
            timing().lease,
        ));
        self.nodes.push(n.clone());
        n
    }

    pub fn tick_all(&self) {
        for n in &self.nodes {
            let _ = n.tick();
        }
    }
}

impl Harness for AuthorityHarness {
    fn nodes(&self) -> Vec<Arc<dyn Membership>> {
        self.nodes
            .iter()
            .map(|n| n.clone() as Arc<dyn Membership>)
            .collect()
    }

    fn settle(&mut self) {
        self.clock.advance_ms(10);
        self.tick_all();
    }

    fn advance(&mut self, d: Duration) {
        self.clock.advance(d);
        self.tick_all();
    }
}

/// A drain that records the shards it drained, in order.
#[derive(Debug, Default)]
pub struct RecordingDrain(pub Vec<ShardId>);

impl Drain for RecordingDrain {
    fn drain_shard(&mut self, shard: ShardId) -> Result<(), String> {
        self.0.push(shard);
        Ok(())
    }
}

/// Acquires on an authority-backed node and asserts success.
pub fn grant(n: &Node, shard: ShardId) -> vertep_cluster::Lease {
    n.acquire(shard).unwrap()
}

/// The authority's current view.
pub fn authority_view(a: &MemoryAuthority) -> vertep_cluster::View {
    a.view().unwrap()
}
