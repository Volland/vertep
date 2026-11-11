## Why

After M2 a Vertep actor system runs on one node, so its capacity and availability end at that node's limits. M4 lets trusted nodes form a cluster that places virtual actors by shard, keeps exactly one activation per actor through failures, routes messages between nodes and spreads stateless workers across them, as decided in [[decisions#Shard-based placement]].

## What Changes

- Add a pluggable membership interface with versioned views, graceful join and leave with drain, failure detection, and shard leases carrying fencing epochs; ship static, gossip (SWIM) and external (etcd, Kubernetes Lease) implementations.
- Add shard placement: a fixed power-of-two shard count chosen at cluster creation, shard = hash of the actor IRI modulo the count, one journal store per shard, activation only on the lease holder, and a single-activation guarantee backed by the lease and by epoch fencing in the journal.
- Add rebalancing by shipping a shard's append-only journal file (v1), optional journal replication to standbys for failover, rebalancing by lease transfer when a networked journal store is used, and an offline resharding procedure that doubles the shard count.
- Add a QUIC cluster transport with mutual TLS, framing, ordering per sender and receiver, backpressure per mailbox, at-least-once delivery with deduplication by message identifier, routing through the shard table, and redirects on stale routing.
- Add worker pools: stateless worker activations placed across nodes, load balancing policies, concurrency limits, no journal, results journaled by the caller, and failure reporting as error events or request timeouts.
- New crate `vertep-cluster`, using `vertep-runtime` and `vertep-store`.

## Capabilities

### New Capabilities
- `cluster-membership`: membership interface, view versions, join, leave with drain, failure detection, static, gossip and external implementations, shard leases with fencing epochs, lease expiry and handover.
- `shard-placement`: fixed shard count, IRI-hash assignment, one journal per shard, activation on the owner only, single activation, journal fencing, forwarding, rebalancing by file shipping, standby replication, lease transfer, resharding.
- `cluster-transport`: QUIC transport as an Event I/O Processor, mutual TLS, framing, per-pair ordering, backpressure, at-least-once with deduplication, shard-table routing, redirects.
- `worker-pools`: stateless worker activations across nodes, load balancing, concurrency limits, no journal, caller-journaled results, failure reporting.

### Modified Capabilities

None. M4 builds on the M2 capabilities `actor-model`, `messaging`, `event-journal` and `storage` without changing their requirements; the fencing epoch on journal appends uses the M2 hook [[openspec:storage#Append Fencing Token]], with the shard IRI as its scope.

## Impact

- **Code:** new crate `vertep-cluster`; the runtime gains hooks for a placement-aware directory and a cluster transport; journal stores accept a fencing epoch on append.
- **Dependencies:** a QUIC implementation, a SWIM gossip library, an etcd client and a Kubernetes client, each behind a cargo feature.
- **tg sections touched:** [[capabilities#Cluster membership]], [[capabilities#Shard placement]], [[capabilities#Cluster transport]], [[capabilities#Worker pools]], [[components#vertep-cluster]], [[roadmap#M4 Cluster]], [[glossary#Shard]], [[glossary#Stateless worker]].
- **Binding decisions:** [[decisions#Shard-based placement]], [[decisions#Event-sourced actors]], [[decisions#IRI identity and lifecycle classes]], [[decisions#Identity, mailbox and transport layers]], [[decisions#Two storage traits with tiramemsu as default]], [[decisions#Lean verification bridge]].
- **Binding constraints:** [[constraints#Tiramemsu has a single writer per file]] constrains the shard-to-file mapping; [[constraints#Relays deliver at least once]] applies to the cluster transport as well.
- **Requirements added:**
  - [[openspec:cluster-membership#Membership Trait]], [[openspec:cluster-membership#Membership View Versions]], [[openspec:cluster-membership#Graceful Join]], [[openspec:cluster-membership#Graceful Leave With Drain]], [[openspec:cluster-membership#Failure Detection]], [[openspec:cluster-membership#Static Membership]], [[openspec:cluster-membership#Gossip Membership]], [[openspec:cluster-membership#External Membership]], [[openspec:cluster-membership#Shard Leases With Fencing Epochs]], [[openspec:cluster-membership#Lease Expiry And Handover]]
  - [[openspec:shard-placement#Fixed Shard Count]], [[openspec:shard-placement#Shard Assignment By Actor IRI Hash]], [[openspec:shard-placement#One Journal File Per Shard]], [[openspec:shard-placement#Activation On Shard Owner Only]], [[openspec:shard-placement#Single Activation Guarantee]], [[openspec:shard-placement#Journal Fencing Backstop]], [[openspec:shard-placement#Message Forwarding To Shard Owner]], [[openspec:shard-placement#Rebalancing By Journal File Shipping]], [[openspec:shard-placement#Journal Replication To Standbys]], [[openspec:shard-placement#Rebalancing By Lease Transfer]], [[openspec:shard-placement#Resharding Procedure]]
  - [[openspec:cluster-transport#QUIC Cluster Transport]], [[openspec:cluster-transport#Mutual TLS Within The Cluster]], [[openspec:cluster-transport#Message Framing]], [[openspec:cluster-transport#Ordering Per Sender And Receiver]], [[openspec:cluster-transport#Flow Control And Backpressure]], [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication]], [[openspec:cluster-transport#Routing Via The Shard Table]], [[openspec:cluster-transport#Redirect On Stale Routing]]
  - [[openspec:worker-pools#Stateless Worker Activations]], [[openspec:worker-pools#Pool Placement Across Nodes]], [[openspec:worker-pools#Load Balancing Policies]], [[openspec:worker-pools#Worker Concurrency Limits]], [[openspec:worker-pools#No Journal For Workers]], [[openspec:worker-pools#Results Journaled By The Caller]], [[openspec:worker-pools#Worker Failure Reporting]]
