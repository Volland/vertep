## Why

An architecture review on 2026-11-16 looked at the runtime, the journal store and the cluster for throughput and for long runs. The semantics are sound: the mailbox contract, routing mailboxes per address, at-least-once delivery with message-id deduplication, and the transactional outbox. The cost of those semantics is not. Nothing in the repository measures it today.

1. **One durable message costs five to seven commits.** A store mailbox inserts five quads when a message arrives, including the whole envelope as JSON. The turn then makes several more commits: the journal append, the transition annotations, the state delta on stores that cannot carry it in the append, and the outbox mark (read, remove, insert). The acknowledgement then removes the five quads. `tg.md/views/persistence.md` says the append acknowledges the message in its own transaction. The code does not.
2. **Deleting does not free space.** Tiramemsu is bitemporal and never deletes a row. A removed mailbox row, a replaced outbox mark and a re-asserted head all stay in the file, with their nine indexes, forever.
3. **An OS crash can lose an entry whose sends were already delivered.** The shard opens with SQLite's `synchronous=NORMAL`, and `after_append` dispatches the sends as soon as the append returns. Receivers can then hold messages that the sender's replayed history does not contain.
4. **One node is one thread with synchronous IO.** Every turn, append and SPARQL lookup runs in sequence inside the async loop. Shards have separate writers, but no node writes to two of them at once. There is no group commit.
5. **Waking an actor costs O(history).** Activation reads and decodes the whole journal from position 0, even when a snapshot exists, and loads every snapshot ever written. The deduplication set holds every message id the actor ever accepted. This is already recorded as debt in [[decisions#Deduplication reads the whole journal on activation]].
6. **Memory and files grow without limit.** Never evicted: the `mailboxes` map, `outbox`, `stats.activations` and `va.seen` in the node; `virtuals`, `PeerConn.pairs` and the replication `acked` map in the cluster. Never pruned: snapshots, `vt:dispatched` marks and dead letters.
7. **Mailbox operations scan the whole queue.** `receive`, `remove`, `release` and `deliverable` walk every row. Deferred messages are matched again on every receive. Mailboxes have no capacity limit.
8. **The QUIC transport stalls after 100 pairs per peer.** quinn's default limit is 100 concurrent bidirectional streams, `max_pair_streams` is 1024, and `open_bi().await` runs while the transport-wide `peers` lock is held. The 101st pair stops every send to every peer, against the scenario `Independent pairs do not block each other`. Per-message retry tasks can also reorder one pair's messages, against `Ordering Per Sender And Receiver`.
9. **The cluster path copies and waits.**
   - Several full `View` clones per message, under the membership mutex.
   - Pair streams are stop-and-wait.
   - Standbys are shipped to sequentially and synchronously, with a pre-flight round trip on every append.

## What Changes

- **New capability `runtime-performance`:**
  - A benchmark harness and a store wrapper that counts operations, with budgets per operation and a regression gate.
  - A long-run soak test that checks memory and storage stay bounded.
  - Throughput and growth limits stated as requirements.
- **`storage`:**
  - A turn commits once: the entry, state, annotations, outbox mark, mailbox acknowledgement and any snapshot in one transaction.
  - Group commit per shard.
  - Bookkeeping kept unversioned, so removing it frees space.
  - Indexed range reads and message-id lookups.
  - Sends are dispatched only after the commit is durable.
- **`event-journal`:**
  - The outbox dispatches after the entry is durable on disk, not merely after it is appended.
  - Activation reads from the latest snapshot.
  - Snapshot retention.
  - Deduplication through an index, with bounded memory.
- **`messaging`:**
  - Receive does not rescan deferred messages.
  - Bounded mailboxes with a declared overflow policy.
  - Opt-in durability by origin: a message an actor sent is held by the sender's journal outbox until the receiver has journaled the turn that consumed it. The receiver's mailbox can then stay in memory, and a message from outside the system is stored durably.
- **`actor-model`:**
  - Shard groups: several single-threaded nodes in one process, each owning a fixed set of shards.
  - Deactivation releases all per-actor memory.
  - Idle actors are found without scanning every active actor.
- **`cluster-transport`:**
  - A stream budget below the peer's limit, with no lock held across connecting or opening a stream.
  - Retries that keep pair order.
  - A window of in-flight frames per pair stream.
- **`shard-placement`:**
  - Standby shipping in parallel.
  - No pre-flight round trip when the standby is already at the local head.

Out of scope, and listed in the design as follow-ups:
- Journal archival to cold storage.
- One lease heartbeat per node instead of per shard.
- Pre-copy hand-off.
- A slimmer cluster frame.

## Capabilities

### New Capabilities

- `runtime-performance`: benchmarks, operation budgets, regression gate, soak test, and the throughput and growth limits.

### Modified Capabilities

- `storage`: one commit per turn, group commit, unversioned bookkeeping, durable commit, range read and message-id lookup.
- `event-journal`: dispatch after a durable commit, activation from the snapshot, snapshot retention, indexed deduplication.
- `messaging`: receive cost under deferral, bounded mailboxes, durability by origin.
- `actor-model`: shard groups, memory released on deactivation, idle detection.
- `cluster-transport`: stream budget, retries that keep pair order, pipelined pair streams.
- `shard-placement`: parallel standby shipping.

## Impact

**Crates:**
- `vertep-runtime`: `mailbox.rs`, `node.rs`, `journal.rs`, a new `group.rs`.
- `vertep-store`: the `JournalStore` additions and a counting wrapper in the test kit.
- `vertep-tiramemsu`: `shard.rs` and `backend.rs`, using tiramemsu's volatile rows and `Durability`.
- `vertep-cluster`: `transport.rs`, `runtime.rs`, `replication.rs` and `view.rs`.
- A new `benches/`, which is also the `production-readiness` benchmark suite (moved here when M13 was archived on 2026-11-17).

**Behaviour visible to applications:**
- `NodeConfig` gains `snapshot_keep`, `durability`, `group_commit` and `shard_groups`.
- `MailboxConfig` routes gain `capacity` and `overflow`.
- `NodeStats.activations` becomes a total, with a per-address view only in debug builds.
- Before 1.0 a minor version may change these APIs.

**Stacking:** this branch carries the `mailbox-backends` commit ("Mailbox backends chosen per address") and its OpenSpec change, so both merge together.

**lat.md sections touched:**
- `roadmap` (new `M18 Runtime performance`)
- `views/persistence#Tiramemsu mapping`, which currently claims an acknowledgement the code does not make
- `views/runtime#Identity, mailbox and transport`
- `views/distribution#Routing over QUIC`
- `components#vertep-runtime`, `components#vertep-tiramemsu`, `components#vertep-cluster`, `components#vertep-store`
- `capabilities#Messaging`, `capabilities#Storage`, `capabilities#Event journal`
- `constraints` (one new constraint, see the design)
- `decisions` (D1 to D12 of the design)

**Decisions that bind the area:**
- [[decisions#A node runs its actors on one thread]]: kept; shard groups are several such nodes.
- [[decisions#Deduplication reads the whole journal on activation]]: superseded.
- [[decisions#Outbox progress lives in an outbox graph]]: superseded.
- [[decisions#Two storage traits with tiramemsu as default]]
- [[decisions#Mailbox backend is chosen per address]]
- [[decisions#QUIC with mutual TLS and a stream per actor pair]]
- [[decisions#At-least-once inside the cluster with journal deduplication]]
- [[decisions#Optional standby journal replication]]
- [[decisions#New milestones wait for production readiness]]: satisfied; the freeze ended when M13 was archived on 2026-11-17.
- [[decisions#Benchmarks measure the claims]]: phase 0 implements it.

**Constraints:**
- [[constraints#Tiramemsu has a single writer per file]]
- [[constraints#QUIC limits streams per connection]]
- [[constraints#Relays deliver at least once]]

**The milestone freeze.** [[decisions#New milestones wait for production readiness]] held new milestones until `m13-production-readiness` was archived. M13 was archived on 2026-11-17 and moved its `Benchmark Suite` requirement into this change, so the freeze no longer applies.

**Requirements added or modified:**
- `production-readiness`:
  - [[openspec:production-readiness#Benchmark Suite]] (moved from M13)
- `runtime-performance`:
  - [[openspec:runtime-performance#Hot Path Benchmarks]]
  - [[openspec:runtime-performance#Operation Budgets]]
  - [[openspec:runtime-performance#Benchmark Regression Gate]]
  - [[openspec:runtime-performance#Bounded Long-Run Growth]]
- `storage`:
  - [[openspec:storage#One Commit Per Turn]]
  - [[openspec:storage#Group Commit Per Shard]]
  - [[openspec:storage#Unversioned Bookkeeping]]
  - [[openspec:storage#Durable Commit]]
  - [[openspec:storage#Journal Range Read]]
  - [[openspec:storage#Message Id Lookup]]
- `event-journal`:
  - [[openspec:event-journal#Transactional Outbox]] (modified)
  - [[openspec:event-journal#Activation Reads From The Snapshot]]
  - [[openspec:event-journal#Snapshot Retention]]
  - [[openspec:event-journal#Indexed Deduplication]]
- `messaging`:
  - [[openspec:messaging#Receive Cost Under Deferral]]
  - [[openspec:messaging#Bounded Mailboxes]]
  - [[openspec:messaging#Durability By Origin]]
- `actor-model`:
  - [[openspec:actor-model#Shard Groups]]
  - [[openspec:actor-model#Deactivation Releases Memory]]
  - [[openspec:actor-model#Idle Detection Without Scanning]]
- `cluster-transport`:
  - [[openspec:cluster-transport#Ordering Per Sender And Receiver]] (modified)
  - [[openspec:cluster-transport#Stream Budget Within The Peer Limit]]
  - [[openspec:cluster-transport#Pipelined Pair Streams]]
- `shard-placement`:
  - [[openspec:shard-placement#Parallel Standby Shipping]]
