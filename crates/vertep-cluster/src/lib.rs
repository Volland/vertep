//! Trusted Vertep clusters: membership, shard leases with fencing epochs, shard placement and
//! the QUIC cluster transport.
//!
//! - [`meta`]: the cluster metadata fixed at creation (identifier, power-of-two shard count,
//!   hash version, replication factor).
//! - [`placement`]: `shard = hash(IRI) mod N`, the split of a shard on resharding, the
//!   deterministic assignment of shards to members, one journal store per shard.
//! - [`lease`]: the lease authority's table (strictly increasing epochs per shard) and the
//!   holder's view of its leases (stop a safety margin before expiry, by its own clock).
//! - [`membership`]: the [`Membership`] trait every implementation provides.
//! - [`authority`]: membership through a lease authority, the static membership and the
//!   in-memory stand-in for external (etcd, Kubernetes) authorities.
//! - `gossip` (feature `gossip`): SWIM membership, sans-IO, with a simulated network.
//! - [`fencing`]: the fenced shard journal, the receiver's epoch filter and dead letters.
//! - [`routing`]: the shard table, redirects, activation on the owner only, forwarding with a
//!   hop limit, deduplication.
//! - [`rebalance`]: the rebalancer's plan, journal shipping, lease transfer and offline
//!   resharding (`vt cluster reshard`).
//! - [`replication`]: standby replication with quorum acknowledgement, and failover.
//! - [`workers`]: the stateless worker pool dispatcher (activations on demand, placement on
//!   declaring members, balancing policies, limits, failure reporting).
//! - [`history`]: fault-injection histories, their checker and their JSON-lines export.
//! - [`frame`]: the cluster wire frame codec; `transport` (feature `quic`): QUIC with mutual
//!   TLS, one stream per sender and receiver pair.
//! - `runtime` (feature `runtime`): the cluster inside `vertep-runtime`: the node's placement,
//!   sharded fenced storage, the cluster Event I/O Processor, outbound retries, inbound
//!   delivery and forwarding, shard drain and worker dispatch.
//!
//! - `external` (features `etcd`, `kubernetes`): the etcd (v3 JSON gateway) and Kubernetes
//!   (`coordination.k8s.io/v1` Lease API) lease authorities, over a small blocking HTTP client.
//!
//! The `vertep-fault-node` executable (feature `fault-node`) is one node of the container
//! fault-injection harness (`tests/containers.rs`).
//!
//! @tg: implements:: [[decisions#Shard-based placement]]
//! @tg: implements:: [[decisions#Shards by IRI hash modulo a fixed count]]
//! @tg: implements:: [[decisions#Leases with fencing epochs from the membership authority]]
//! @tg: implements:: [[decisions#Local shard journals shipped on move]]
//! @tg: implements:: [[decisions#Fencing on appends and outbound frames]]
//! @tg: implements:: [[decisions#Membership behind one trait]]
//! @tg: implements:: [[decisions#QUIC with mutual TLS and a stream per actor pair]]
//! @tg: implements:: [[decisions#At-least-once inside the cluster with journal deduplication]]
//! @tg: implements:: [[decisions#Rebalancing moves the fewest shards and resharding is offline]]
//! @tg: implements:: [[decisions#Workers are pooled, not sharded]]
//! @tg: implements:: [[decisions#A free shard goes only to a node holding its journal]]
//! @tg: implements:: [[decisions#Unreplicated entries stay unreadable until the quorum catches up]]
//! @tg: implements:: [[decisions#Worker pools dispatch from the sending node]]
//! @tg: implements:: [[decisions#Fault-injection histories export as JSON lines]]
//! @tg: implements:: [[decisions#Offline resharding reads a lease dump]]
//! @tg: implements:: [[decisions#Fencing epochs on journal appends]]
//! @tg: implements:: [[decisions#Power-of-two shard counts]]
//! @tg: implements:: [[decisions#Single-node shards placed by FNV-1a of the address IRI]] {note: "the cluster hash version 1 is the same FNV-1a placement"}
//! @tg: implements:: [[decisions#Stores without fencing refuse tokens]] {note: "a shard journal refuses a store without fencing"}
//! @tg: implements:: [[decisions#External authorities over etcd and the Kubernetes Lease API]]
//! @tg: implements:: [[decisions#etcd expires shard grants and a permanent key holds each shard's epoch]]
//! @tg: implements:: [[decisions#Kubernetes grants expire by observation]]
//! @tg: implements:: [[decisions#An owner checks its standbys before writing its own copy]]
//! @tg: implements:: [[decisions#A standby's fence rises only with entries it stores]]
//! @tg: implements:: [[decisions#A new owner replicates to the shard's current standbys]]
//! @tg: implements:: [[decisions#Gossip leader epochs carry a per-start tag]]
//! @tg: implements:: [[decisions#A leaving node ships each shard before handing it over]]
//! @tg: implements:: [[decisions#Frames to other nodes are kept until settled]]

pub mod authority;
pub mod clock;
pub mod fencing;
pub mod frame;
pub mod history;
pub mod lease;
pub mod membership;
pub mod meta;
pub mod placement;
pub mod rebalance;
pub mod replication;
pub mod routing;
pub mod view;
pub mod workers;

#[cfg(feature = "gossip")]
pub mod gossip;

#[cfg(any(feature = "etcd", feature = "kubernetes"))]
pub mod external;

#[cfg(feature = "quic")]
pub mod transport;

#[cfg(feature = "runtime")]
pub mod runtime;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use authority::{
    Authority, AuthorityConfig, AuthorityLink, AuthorityMembership, ExternalMembership,
    MemoryAuthority, StaticConfig, StaticMembership,
};
pub use clock::{Clock, ManualClock, Millis, MonotonicClock};
pub use fencing::{
    DeadLetters, EpochFilter, Fence, FencedMessage, MemoryDeadLetters, ShardJournal,
    ShardJournalError,
};
pub use frame::{Frame, FrameError, Reply};
pub use history::{AppendDecision, CheckReport, FrameDecision, History, HistoryEvent, Violation};
pub use lease::{EpochPolicy, HeldLeases, Lease, LeaseConfig, LeaseError, LeaseTable};
pub use membership::{
    DetectorConfig, Drain, JoinError, JoinRequest, Membership, MembershipError, check_compatible,
};
pub use meta::{ClusterId, ClusterMeta, HashVersion, MetaError, NodeId, ShardCount};
pub use placement::{ShardId, ShardStores, assignment, shard_of};
pub use rebalance::{
    Move, ReshardError, ReshardReport, ShipError, ShipReport, move_by_lease_transfer,
    move_by_shipping, plan_moves, reshard_offline, ship_journal, verify_same_end,
};
pub use replication::{
    LocalStandby, ReplicaError, ReplicaLink, ReplicatedJournal, ReplicationBatch, ShipOutcome,
    StandbyLink, StandbyReplica, may_take, replica_set, ship_suffix, standbys_of,
};
pub use routing::{ActivationGate, DedupCache, Inbound, NotOwner, Redirect, RoutingTable};
pub use view::{Member, MemberStatus, ShardOwner, Subscription, View, ViewTracker};
pub use workers::{
    ActivationId, Assignment, Failure, Outcome, Policy, Refusal, Submit, WorkerPool, WorkerType,
};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
