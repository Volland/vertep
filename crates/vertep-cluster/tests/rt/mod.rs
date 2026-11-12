//! A cluster of `vertep-runtime` nodes for the integration tests: a static membership on a
//! manual lease clock, one runtime node per member on cluster storage, joined by a wire.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use vertep_cluster::runtime::{ClusterConfig, ClusterNode, ClusterStorage, MemoryWire, Wire};
use vertep_cluster::{
    AuthorityMembership, ClusterMeta, LocalStandby, ManualClock, Membership, MemoryAuthority,
    NodeId, ShardId, StandbyLink, StandbyReplica, StaticConfig, replica_set, standbys_of,
};
use vertep_core::datamodel::{Event, EventType};
use vertep_core::ir::Chart;
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{Address, Node, Shell, Storage};
use vertep_store::MemoryStore;

use crate::common::{meta, node, timing};

pub type StaticNode = AuthorityMembership<Arc<MemoryAuthority>>;

/// A chart type IRI.
pub fn ty(name: &str) -> String {
    format!("https://example.org/charts/{name}")
}

/// A virtual actor address.
pub fn va(name: &str, key: &str) -> Address {
    Address::virtual_actor(&ty(name), key).unwrap()
}

/// An SCXML chart with the given datamodel (`null` or `ecmascript`).
pub fn scxml(datamodel: &str, initial: &str, body: &str) -> Chart {
    let xml = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="{datamodel}" initial="{initial}">{body}</scxml>"#
    );
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(&xml, &opts).unwrap().chart
}

/// An external event.
pub fn ev(name: &str) -> Event {
    Event::named(name, EventType::External)
}

/// The input events journaled for `a`.
pub fn inputs(node: &Node, a: &Address) -> Vec<Event> {
    node.journal(a)
        .unwrap_or_default()
        .iter()
        .filter_map(|e| match &e.body {
            EntryBody::Input { input, .. } => Some(input.event.clone()),
            _ => None,
        })
        .collect()
}

/// One member: its membership, its cluster side, its storage and its runtime node.
pub struct Member {
    pub id: NodeId,
    pub membership: Arc<StaticNode>,
    pub cluster: Arc<ClusterNode>,
    pub storage: Arc<ClusterStorage<Arc<MemoryStore>>>,
    pub node: Node,
    /// The member's store of each shard.
    pub stores: Vec<Arc<MemoryStore>>,
    /// The member's standby replica of each shard (over the same stores).
    pub replicas: Vec<Arc<StandbyReplica<Arc<MemoryStore>>>>,
    /// The links the member's owned shards replicate over, by shard.
    pub links: Vec<Vec<Arc<LocalStandby<Arc<MemoryStore>>>>>,
}

/// A runtime cluster.
pub struct Cluster {
    pub config: ClusterConfig,
    pub meta: ClusterMeta,
    pub clock: ManualClock,
    pub authority: Arc<MemoryAuthority>,
    pub wire: Arc<MemoryWire>,
    pub members: Vec<Member>,
}

/// Builds the runtime node of a member.
pub fn runtime_node(
    membership: Arc<StaticNode>,
    cluster: &Arc<ClusterNode>,
    storage: Arc<ClusterStorage<Arc<MemoryStore>>>,
) -> Node {
    let node = Node::builder(storage as Arc<dyn Storage>, Shell::tokio())
        .datamodels(Arc::new(vertep_dm_ecma::EcmaRegistry))
        .placement(cluster.placement())
        .transport(cluster.processor())
        .build();
    cluster.attach(node.handle());
    let _ = membership;
    node
}

impl Cluster {
    /// A static cluster of `n` members over `shards` shards, joined by an in-memory wire,
    /// each member holding the shards the deterministic assignment gives it.
    pub fn new(n: usize, shards: u64) -> Self {
        Self::with_config(n, shards, ClusterConfig::default())
    }

    /// The same, with `config` on every node.
    pub fn with_config(n: usize, shards: u64, config: ClusterConfig) -> Self {
        Self::build(n, shards, config, 1)
    }

    /// A cluster with replication factor `rf`: each shard's owner (by the deterministic
    /// assignment of the initial view) replicates to the next `rf - 1` members and
    /// acknowledges an append once all of them stored it.
    pub fn replicated(n: usize, shards: u64, rf: u32) -> Self {
        Self::build(n, shards, ClusterConfig::default(), rf)
    }

    fn build(n: usize, shards: u64, config: ClusterConfig, rf: u32) -> Self {
        let clock = ManualClock::new();
        let static_config = StaticConfig {
            meta: meta(shards).with_replication_factor(rf).unwrap(),
            members: (0..n)
                .map(|i| (node(&format!("n{i}")), format!("127.0.0.1:{}", 7400 + i)))
                .collect(),
            timing: timing(),
        };
        let authority = Arc::new(static_config.authority(Arc::new(clock.clone())));
        let wire = MemoryWire::new();
        let ids: Vec<NodeId> = static_config
            .members
            .iter()
            .map(|(id, _)| id.clone())
            .collect();
        let stores: Vec<Vec<Arc<MemoryStore>>> = ids
            .iter()
            .map(|_| (0..shards).map(|_| Arc::new(MemoryStore::new())).collect())
            .collect();
        let replicas: Vec<Vec<Arc<StandbyReplica<Arc<MemoryStore>>>>> = stores
            .iter()
            .map(|per| {
                per.iter()
                    .enumerate()
                    .map(|(s, st)| {
                        Arc::new(StandbyReplica::new(
                            static_config.meta.clone(),
                            ShardId::new(s as u32),
                            st.clone(),
                        ))
                    })
                    .collect()
            })
            .collect();
        let view = vertep_cluster::Authority::view(&*authority).unwrap();
        let sets = replica_set(&view, static_config.meta.shard_count, rf);
        let mut members = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let membership = Arc::new(
                static_config
                    .membership(id, authority.clone(), Arc::new(clock.clone()))
                    .unwrap(),
            );
            membership.poll().unwrap();
            let cluster = ClusterNode::new(membership.clone() as Arc<dyn Membership>, config);
            wire.connect(&cluster);
            // a link from this member to every member's replica of every shard
            let to: Vec<Vec<Arc<LocalStandby<Arc<MemoryStore>>>>> = ids
                .iter()
                .enumerate()
                .map(|(j, other)| {
                    (0..shards as usize)
                        .map(|s| Arc::new(LocalStandby::new(other.clone(), replicas[j][s].clone())))
                        .collect()
                })
                .collect();
            // the links of the shards it owns from the start
            let mut links: Vec<Vec<Arc<LocalStandby<Arc<MemoryStore>>>>> =
                vec![Vec::new(); shards as usize];
            for (s, set) in &sets {
                if set.first() == Some(id) {
                    for standby in &set[1..] {
                        let j = ids.iter().position(|x| x == standby).unwrap();
                        links[s.index()].push(to[j][s.index()].clone());
                    }
                }
            }
            let required = rf.saturating_sub(1) as usize;
            // whenever it gets a new lease on a shard: the shard's current standbys
            let (current, me, all) = (membership.clone(), id.clone(), ids.clone());
            let storage = Arc::new(
                ClusterStorage::with_standbys(
                    membership.clone() as Arc<dyn Membership>,
                    stores[i].clone(),
                    move |s| {
                        standbys_of(&current.view(), &me, rf)
                            .iter()
                            .map(|n| {
                                let j = all.iter().position(|x| x == n).unwrap();
                                to[j][s.index()].clone() as Arc<dyn StandbyLink>
                            })
                            .collect()
                    },
                    required,
                )
                .unwrap(),
            );
            cluster.keep_outbound(storage.clone() as Arc<dyn Storage>);
            let node = runtime_node(membership.clone(), &cluster, storage.clone());
            members.push(Member {
                id: id.clone(),
                membership,
                cluster,
                storage,
                node,
                stores: stores[i].clone(),
                replicas: replicas[i].clone(),
                links,
            });
        }
        let mut c = Cluster {
            config,
            meta: static_config.meta.clone(),
            clock,
            authority,
            wire,
            members,
        };
        c.claim_all();
        c
    }

    /// Every member claims its assigned shards; every member then learns the owners.
    pub fn claim_all(&mut self) {
        for m in &self.members {
            let storage = m.storage.clone();
            m.cluster.claim(&|s| storage.holds_journal(s));
        }
        for m in &self.members {
            let _ = m.membership.poll();
            m.cluster.refresh();
        }
    }

    /// Registers chart type `name` on every member.
    pub fn register(&mut self, name: &str, chart: &Chart) {
        for m in &mut self.members {
            m.node.register_type(&ty(name), chart.clone()).unwrap();
        }
    }

    /// The index of the member owning `a`'s shard.
    pub fn owner_of(&self, a: &Address) -> usize {
        let s = self.meta.shard_of(a.as_str());
        self.members
            .iter()
            .position(|m| m.membership.held(s).is_some())
            .expect("an owner")
    }

    /// The shard of `a`.
    pub fn shard(&self, a: &Address) -> ShardId {
        self.meta.shard_of(a.as_str())
    }

    /// A key of chart type `name` whose actor lives on member `i`.
    pub fn actor_on(&self, name: &str, i: usize, tag: &str) -> Address {
        (0..)
            .map(|k| va(name, &format!("{tag}-{k}")))
            .find(|a| self.owner_of(a) == i)
            .unwrap()
    }

    /// Runs every node until `done` holds, for at most `max` of wall time.
    pub async fn until(
        &mut self,
        max: Duration,
        mut done: impl FnMut(&mut Cluster) -> bool,
    ) -> bool {
        let start = std::time::Instant::now();
        loop {
            for m in &mut self.members {
                m.node.run_until_idle();
            }
            if done(self) {
                return true;
            }
            if start.elapsed() > max {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Runs every node for `d` of wall time.
    pub async fn run_for(&mut self, d: Duration) {
        self.until(d, |_| false).await;
    }

    /// Advances the lease clock and lets every member tick.
    pub fn advance(&mut self, d: Duration) {
        let all: Vec<usize> = (0..self.members.len()).collect();
        self.advance_only(d, &all);
    }

    /// Advances the lease clock in steps of one second, letting only members `alive` tick
    /// (the others are crashed or paused).
    pub fn advance_only(&mut self, d: Duration, alive: &[usize]) {
        let mut left = d;
        while !left.is_zero() {
            let step = left.min(Duration::from_secs(1));
            left -= step;
            self.clock.advance(step);
            let _ = self.authority.tick();
            for &i in alive {
                let _ = self.members[i].membership.tick();
                self.members[i].cluster.refresh();
            }
        }
    }

    /// Restarts member `i` as a new process over the same membership and storage: its
    /// cluster side and runtime node are dropped (with whatever they held in memory) and built
    /// again; chart types must be registered again.
    pub fn restart(&mut self, i: usize) -> Arc<ClusterNode> {
        let m = &mut self.members[i];
        m.cluster.stop();
        let cluster = ClusterNode::new(m.membership.clone() as Arc<dyn Membership>, self.config);
        self.wire.connect(&cluster);
        let node = runtime_node(m.membership.clone(), &cluster, m.storage.clone());
        m.cluster = cluster.clone();
        m.node = node;
        let storage = m.storage.clone();
        cluster.keep_outbound(storage.clone() as Arc<dyn Storage>);
        cluster.claim(&|s| storage.holds_journal(s));
        cluster.refresh();
        cluster.resume_outbound();
        cluster
    }

    /// The wire as a trait object.
    pub fn wire(&self) -> Arc<dyn Wire> {
        self.wire.clone()
    }
}
