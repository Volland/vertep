<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-cluster</h1>

<p align="center">Trusted Vertep clusters: membership, shard leases with fencing epochs, a QUIC transport and worker pools</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-cluster"><img src="https://img.shields.io/crates/v/vertep-cluster.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-cluster"><img src="https://img.shields.io/docsrs/vertep-cluster" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-cluster.svg" alt="license"></a>
</p>

This crate runs one population of Vertep actors on several trusted machines. Actors are placed
by shard: `shard = hash(IRI) mod N`, with a fixed power-of-two shard count. A node owns a shard
through a time-bounded lease, and every new lease carries a strictly greater fencing epoch.
Every journal append carries the owner's epoch, so a node that lost its lease (a long pause,
clock skew) cannot write: its append is rejected as `Fenced`. The Lean model in the repository
proves that at most one writer's appends are accepted per shard and epoch.

Membership sits behind one `Membership` trait, with static, SWIM gossip, etcd and Kubernetes
implementations. `ClusterNode` (feature `runtime`) joins a `vertep-runtime` node to the
cluster: it is the node's placement, its fenced storage and its `<send type="cluster">` Event
I/O Processor. Nodes talk over QUIC with mutual TLS (feature `quic`). Stateless worker pools
are dispatched from the sending node. The crate builds on `vertep-store` and, with its
features, on `vertep-runtime` and `vertep-tiramemsu`; `vt cluster reshard` in `vertep-cli` uses
it. The `vertep` facade's `cluster` feature turns on this crate with `gossip`, `quic` and
`runtime` and re-exports it as `vertep::cluster`. Depend on it directly for the etcd or
Kubernetes authorities, which the facade does not enable.

## Install

```toml
[dependencies]
vertep-cluster = "0.1"   # default features: gossip, quic, runtime
# with external lease authorities:
# vertep-cluster = { version = "0.1", features = ["etcd", "kubernetes"] }
```

| Feature | Turns on |
| --- | --- |
| `gossip` (default) | `gossip`: SWIM membership, sans-IO, with an in-process simulated network |
| `quic` (default) | `transport`: the QUIC cluster transport with mutual TLS (quinn, rustls, rcgen) |
| `runtime` (default) | `runtime`: `ClusterNode`, `ClusterStorage`, the cluster Event I/O Processor, `MemoryWire` and `QuicWire` |
| `etcd` | `external::etcd::EtcdAuthority`: leases over the etcd v3 JSON gateway |
| `kubernetes` | `external::kubernetes::KubeAuthority`: leases as `coordination.k8s.io/v1` Lease objects |
| `tiramemsu` | `rebalance::reshard_directory`: offline resharding of a directory of tiramemsu shard files, as `vt cluster reshard` runs it |
| `testkit` | `testkit`: the shared membership test kit |
| `differential`, `fault-node` | for the repository's own tests: require the Lean lease oracle; build the `vertep-fault-node` container harness binary |

## Example

```rust
use std::sync::Arc;
use std::time::Duration;

use vertep_cluster::{
    AuthorityConfig, ClusterId, ClusterMeta, ManualClock, Membership, NodeId, ShardJournal,
    StaticConfig,
};
use vertep_store::oxrdf::{Literal, NamedNode, Triple};
use vertep_store::{AppendError, AppendMeta, Entry, JournalStore, MemoryStore, Position};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // A static cluster of two nodes and 8 shards. Leases last 10 s by default.
    let n0 = NodeId::new("n0")?;
    let n1 = NodeId::new("n1")?;
    let config = StaticConfig {
        meta: ClusterMeta::new(ClusterId::new("demo")?, 8)?,
        members: vec![(n0.clone(), "10.0.0.1:7400".into()), (n1.clone(), "10.0.0.2:7400".into())],
        timing: AuthorityConfig::default(),
    };
    // Separate clocks: the lease authority's, and each node's own.
    let authority_clock = ManualClock::new();
    let authority = Arc::new(config.authority(Arc::new(authority_clock.clone())));
    let (clock0, clock1) = (ManualClock::new(), ManualClock::new());
    let node0 = Arc::new(config.membership(&n0, authority.clone(), Arc::new(clock0.clone()))?);
    let node1 = Arc::new(config.membership(&n1, authority.clone(), Arc::new(clock1.clone()))?);

    let actor = NamedNode::new("https://example.org/charts/order/42")?;
    let shard = config.meta.shard_of(actor.as_str());
    let entry = |n: u32| -> Result<Entry, Box<dyn std::error::Error>> {
        let name = NamedNode::new(format!("{}/entry/{n}", actor.as_str()))?;
        Ok(Entry::new(name.clone(), [Triple::new(name, NamedNode::new("urn:example:n")?, Literal::from(n))]))
    };

    // Both nodes share one journal store that enforces fencing.
    let store = Arc::new(MemoryStore::new());
    let journal0 = ShardJournal::new(store.clone(), shard, node0.clone())?;
    let journal1 = ShardJournal::new(store.clone(), shard, node1.clone())?;

    // n0 takes the shard at epoch 1 and appends.
    let lease = node0.acquire(shard)?;
    journal0.append(&actor, Position::ZERO, &[entry(1)?], &AppendMeta::none())?;

    // n0 stalls: its own clock stops and it sends no heartbeats. n1 keeps ticking (heartbeat,
    // poll the view, renew) while the authority's clock moves past n0's lease expiry.
    for _ in 0..11 {
        authority_clock.advance(Duration::from_secs(1));
        clock1.advance(Duration::from_secs(1));
        node1.tick()?;
    }
    let newer = node1.acquire(shard)?;
    assert!(newer.epoch > lease.epoch);
    journal1.append(&actor, Position::new(1), &[entry(2)?], &AppendMeta::none())?;

    // n0 wakes up still believing it holds the lease. Its append carries the old epoch.
    let err = journal0
        .append(&actor, Position::new(2), &[entry(3)?], &AppendMeta::none())
        .unwrap_err();
    assert!(matches!(err, AppendError::Fenced { .. }));
    assert!(node0.held(shard).is_none()); // n0 dropped the lease and stops serving the shard
    assert_eq!(store.head(&actor)?, Position::new(2));
    Ok(())
}
```

Node `n0` takes a shard's lease at epoch 1 and appends. Then it stalls: its own clock does not
move, while the authority's clock passes the lease's expiry, so `n1` takes the shard at a
higher epoch. When `n0` wakes, it still thinks its lease is valid, but the store has already
seen the higher epoch and rejects the append as `Fenced`. `n0` then forgets the lease. The
journal stays linear with one writer per epoch. In a real node `ClusterStorage` does this for
every actor turn, and `ClusterNode` routes each message to the shard's owner.

Joining a runtime node to the cluster (feature `runtime`) follows the crate's runtime tests.
It compiles as is; it is not run here because a node needs a running tokio runtime:

```rust,no_run
use std::sync::Arc;

use vertep_cluster::Membership;
use vertep_cluster::runtime::{ClusterConfig, ClusterNode, ClusterStorage, MemoryWire};
use vertep_runtime::{Node, Shell, Storage};
use vertep_store::MemoryStore;

/// A runtime node whose placement, storage and `<send type="cluster">` come from the cluster.
fn clustered_node(
    membership: Arc<dyn Membership>,
    wire: &Arc<MemoryWire>, // in-memory frames for tests; `QuicWire` in production
) -> Result<Node, Box<dyn std::error::Error>> {
    let shard_count = membership.meta().shard_count.shards().count();
    let stores: Vec<Arc<MemoryStore>> = (0..shard_count).map(|_| Arc::new(MemoryStore::new())).collect();
    let storage = Arc::new(ClusterStorage::new(membership.clone(), stores)?);

    let cluster = ClusterNode::new(membership, ClusterConfig::default());
    wire.connect(&cluster);
    let node = Node::builder(storage as Arc<dyn Storage>, Shell::tokio())
        .placement(cluster.placement())
        .transport(cluster.processor())
        .build();
    cluster.attach(node.handle());
    Ok(node)
}
```

## Key types

| Item | What it is |
| --- | --- |
| `ClusterMeta`, `ClusterId`, `NodeId`, `ShardCount`, `HashVersion` | the metadata fixed at creation: id, power-of-two shard count, hash version (1 is FNV-1a, the same as `vertep-tiramemsu`), replication factor |
| `shard_of`, `assignment`, `ShardId`, `ShardStores` | placement: shard of an IRI, deterministic shard-to-member assignment, one journal store per shard |
| `Membership` | the trait every membership implements: `view`, `subscribe`, `acquire`, `renew`, `release`, `revoke`, `handover`, `held` |
| `StaticConfig`, `StaticMembership`, `MemoryAuthority`, `AuthorityMembership` | a static member list with a lease authority hosted by one process |
| `gossip::GossipMembership` | SWIM gossip membership with a deterministic leader (feature `gossip`) |
| `Lease`, `LeaseTable`, `LeaseConfig`, `HeldLeases` | leases with strictly increasing epochs; the holder stops a safety margin before expiry |
| `ShardJournal`, `EpochFilter`, `Fence`, `DeadLetters` | fenced appends; the receiver's filter that drops frames from stale epochs into dead letters |
| `RoutingTable`, `Redirect`, `DedupCache` | shard table routing, forwarding with a hop limit, deduplication |
| `ReplicatedJournal`, `StandbyReplica` | standby replication with quorum acknowledgement, and failover |
| `plan_moves`, `ship_journal`, `move_by_shipping`, `move_by_lease_transfer`, `reshard_offline` | rebalancing and offline resharding |
| `WorkerPool`, `WorkerType`, `Policy` | stateless worker pools: `RoundRobin`, `LeastOutstanding`, `PreferLocal` |
| `History`, `CheckReport` | fault-injection histories, their checker and JSON-lines export |
| `runtime::ClusterNode`, `ClusterStorage`, `ClusterConfig`, `MemoryWire`, `QuicWire` | the cluster inside a runtime node (feature `runtime`) |
| `transport::ClusterTransport`, `ClusterCa`, `NodeIdentity` | QUIC with mutual TLS, one stream per sender and receiver pair (feature `quic`) |

## Known gaps

- Journal shipping between nodes has no wire protocol yet. The shipping hook accepts any copy
  of a journal, and the tests ship between stores in one process. To move a shard between
  machines, you supply the way its file is copied.
- A cluster no larger than its replication factor stops writing while a node is down: appends
  wait rather than acknowledge with fewer copies than configured.
- Hot shards are not split; individual actors are never moved off a busy shard.
- Resharding is offline only (`vt cluster reshard` on a stopped cluster).
- Gossip lease granting is weaker than consensus under partitions. Fencing keeps it safe, at the
  cost of availability.

## Documentation

- Website: https://volland.github.io/vertep/ ([Cluster and federation](https://volland.github.io/vertep/docs/distribution.html#cluster), [Verification](https://volland.github.io/vertep/docs/verification.html))
- API reference: https://docs.rs/vertep-cluster
- Examples: https://volland.github.io/vertep/examples/

## License

MIT OR Apache-2.0, at your option.
