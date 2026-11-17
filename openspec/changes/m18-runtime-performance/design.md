## Context

The review evidence, with paths as of `main` at 546b54c plus the mailbox-backends commit:

- **Turn path** (`vertep-runtime/src/node.rs`):
  - `virtual_turn` appends, then acknowledges the mailbox.
  - It then writes annotations (`write_annotations`) and advances the outbox mark (`mark_through`: select, remove, insert).
  - It writes a snapshot every 100 positions, each a separate commit.
  - `load` calls `read(actor, Position::ZERO)`, decodes every entry and loads every snapshot.
  - `seen` collects every message id.
- **Store mailbox** (`mailbox.rs`):
  - Five quads per message, with the envelope as a JSON literal, inserted on enqueue and removed on acknowledgement.
  - `Rows::receive`, `remove`, `release` and `deliverable` are linear scans.
  - Nothing bounds a mailbox.
- **Tiramemsu** (0.12):
  - A bitemporal SQLite store in WAL mode with one writer per file, opened at `Durability::Normal`.
  - Retraction sets `t_ret` and keeps the row; triggers forbid `DELETE`.
  - The `triple` table has nine indexes.
  - The *volatile* table (`Tx::set_volatile` / `clear_volatile`, `Store::volatile_all`) is unversioned.
  - `read` is a SPARQL join over every entry of the actor, filtered by position.
- **Cluster** (`vertep-cluster`):
  - `ClusterTransport::stream` holds the `peers` tokio mutex across `connect` and `open_bi`.
  - `max_pair_streams = 1024`, while quinn's default `max_concurrent_bidi_streams` is 100.
  - Every outbound message gets its own retry task (every 50 ms for up to 30 s).
  - `refresh()` clones the whole `View` before it checks the version.
  - `StandbySet::replicate` ships to standbys one after another, with a pre-flight round trip.
- **No benchmarks** exist (`m13-production-readiness` task 4.1 is open).

## Goals / Non-Goals

**Goals:**

- One commit per turn, and one disk sync per batch of turns on a shard.
- Throughput grows with the number of shards within one process.
- Activation cost depends on the entries since the latest snapshot, not on history.
- Memory per deactivated actor is zero, and storage per acknowledged message is the journal entry and nothing else.
- No cluster-wide stall from one pair, one peer or one slow handshake.
- Every claim above has a benchmark or a counted-operation test behind it.

**Non-Goals:**

- Truncating or archiving the journal. Explanation and audit need the full history; archival to cold storage is a follow-up change.
- Changing the journal entry format, the replay rules or the digest.
- One lease heartbeat per node instead of per shard; pre-copy hand-off; a slimmer cluster frame. These are follow-ups listed under Open Questions.
- Making datamodels `Send`. Shard groups keep each activation on one thread.

## Decisions

### D1. Measure first: a benchmark harness and a store that counts

Phase 0 adds `benches/runtime` (criterion) with these cases:
- Turns per second, with a memory and a tiramemsu store, with and without a send.
- Activation time against journal length (10, 1k and 100k entries), with and without a snapshot.
- Mailbox receive with 0, 100 and 10k deferred messages.
- Cross-node send round trip.

It also adds `CountingStorage` to the `vertep-store` test kit. The wrapper counts commits, quads written and read, SPARQL queries and journal reads per call. The budgets in [[openspec:runtime-performance#Operation Budgets]] are asserted against those counts in ordinary tests, so they hold on any machine. Wall-clock numbers go to the regression gate and the results page that m13 D3 already plans.

- motivated_by:: [[decisions#New milestones wait for production readiness]] {weight: primary, note: "the evidence m13 asks for"}

Rejected: tuning without a baseline, and budgets in wall-clock time only (they flake on shared CI runners).

### D2. Effects follow a durable commit

The journal shard opens with `Durability::Immediate` by default, so a commit is synced to disk before `append` returns, and only then are its sends dispatched. `NodeConfig::durability = Relaxed` is an explicit opt-in. Its documented risk is that an OS crash can lose the newest entries after their sends were delivered. A relaxed store declares that in `JournalCapabilities::durable_commit = false`.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary}
- motivated_by:: [[constraints#SQLite NORMAL synchronous can lose the newest commits]] {weight: primary}

Group commit (D5) pays for the sync. Rejected: keeping `Normal` and flushing before dispatch, which is a second sync per turn.

### D3. One transaction per turn

`AppendMeta` gains:
- `annotations`: the reified transition statements now written by `write_annotations`.
- `outbox_through`: the new mark.
- `ack`: the message id to remove from a mailbox in the same shard.
- `snapshot`: when the interval falls due.

Tiramemsu writes all of them inside the one `Db::transact` that checks the head, so a turn is one commit and is all or nothing. A store that cannot hold one of these returns it as unsupported, and the node falls back to separate writes, as it does for state today. This makes the claim in `views/persistence#Tiramemsu mapping` true.

- motivated_by:: [[decisions#State changes travel in the append]] {weight: primary}
- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: secondary, note: "one writer makes one transaction per turn cheap"}

Rejected: batching the side writes later, which reopens the window between append and acknowledgement.

### D4. Bookkeeping is unversioned

These are not facts anyone queries over time:
- mailbox rows,
- the outbox mark,
- `vt:dispatched` timer marks,
- the head,
- snapshots (recomputable by replay).

They move from bitemporal triples to tiramemsu's volatile table, keyed by actor and kind, where clearing a row frees it. Journal entries, current state, annotations and dead letters stay RDF, because they are the record and are queried with SPARQL. A mailbox row stores the envelope once, as compact JSON. The payload is no longer also written as `vt:payload`.

- supersedes:: [[decisions#Outbox progress lives in an outbox graph]] {reason: "an outbox mark replaced every turn leaves a versioned row each time"}
- motivated_by:: [[constraints#Tiramemsu canonicalises literals]] {weight: secondary, note: "envelope literals become permanent term rows"}

Rejected:
- A second SQLite file for bookkeeping: two writers, and no shared transaction with the append.
- Physical deletes in tiramemsu: against its design.

### D5. Group commit per shard

The node gathers the turns of one scheduling round by shard and appends each shard's batch with `JournalStore::append_batch(&[(actor, expected, entries, meta)])`. The call returns one result per actor. Inside the single transaction, tiramemsu checks every head first, under its single writer. It skips the actors that conflict or are fenced, writes the rest, and syncs once. Acknowledgements and dispatch happen per actor only after that commit. Determinism is unchanged: each actor's turns stay in order, and batching only changes when the commit lands.

- motivated_by:: [[constraints#Tiramemsu has a single writer per file]] {weight: primary, note: "heads cannot move between the check and the write inside one transaction"}
- motivated_by:: [[decisions#Fencing epochs on journal appends]] {weight: secondary}

The batch is bounded by `group_commit.max_turns` (default 64) and `group_commit.max_wait_ms` (default 0, which means only what is already ready). Rejected: SQLite savepoints per actor, which add work without changing the outcome under one writer.

### D6. Shard groups: several single-threaded nodes in one process

`NodeGroup::new(storage, shell, k)` runs k `Node`s, each on its own thread with its own `LocalSet`. Node i owns the shards where `shard % k == i`. Activations never cross threads, so datamodels that are not `Send` still work. A local send to an actor in another group goes to that group's inbox: a `Send` channel of envelopes plus `Notify`. The default k = 1 keeps today's behaviour and the virtual-clock tests. Shard writers are already separate, so k groups append in parallel.

- motivated_by:: [[decisions#A node runs its actors on one thread]] {weight: primary, note: "kept: each group is such a node"}
- motivated_by:: [[decisions#Single-node shards placed by FNV-1a of the address IRI]] {weight: primary}

Rejected:
- A work-stealing pool of activations: needs `Send` datamodels and breaks single-thread determinism.
- Moving storage calls to `spawn_blocking` in one node: overlaps IO, but turns would still not run in parallel.

### D7. Activation reads from the snapshot, and deduplication uses an index

`JournalStore` already has `read_range(actor, from, limit)`, added for journal shipping in M13 (`m13-production-readiness`). It gains:
- `latest_snapshot(actor) -> Option<SnapshotRecord>`
- `contains_message(actor, id) -> bool`

Each has a default implementation over `read`, so other backends keep working, and the tiramemsu `read_range` is checked against the index budget rather than rewritten unless the benchmark shows it scans. Tiramemsu answers them from indexes:
- the position on the entry,
- the snapshot row in the volatile table,
- a volatile row per accepted message id, written in the append transaction.

Activation loads the snapshot and reads only the entries after it. The outbox replay reads only the entries after the mark. Deduplication checks a bounded in-memory window (the ids accepted since the activation, at most `dedup_window`) and falls back to `contains_message` otherwise. The guarantee stays exact: each id is accepted at most once per actor.

- supersedes:: [[decisions#Deduplication reads the whole journal on activation]] {reason: "its own note names the index as the planned fix"}

Rejected: per-sender sequence watermarks. Deferral accepts one sender's messages out of order, so a watermark needs a hole set. Messages from outside the system have no sender sequence at all.

### D8. Mailbox internals: queue, stash and capacity

The in-memory mailbox becomes:
- a FIFO `ready` queue;
- a `stash` of deferred messages in arrival order;
- one `in_flight` slot for a virtual actor, or a map by id for worker pools;
- a map from id to sequence number for acknowledgement.

`receive(deferred)` pops from `ready`. A message that matches `deferred` moves to the stash and is not matched again until the receiver's deferring descriptors change. The node signals that change with `Mailbox::redefer`, and the stash then merges back by sequence number, keeping the order [[openspec:messaging#Deferred Events]] requires. `has_deliverable` answers whether any message is deliverable, without counting them. `len` stops cloning envelopes.

A route may declare `capacity` and `overflow: reject | wait`:
- `reject` returns `MailboxError::Full`. The outbox treats it as a transport refusal and retries after back-off. An outside `send` returns `DeliveryError::Full`.
- `wait` applies only to the cluster transport's backpressure, which [[openspec:cluster-transport#Flow Control And Backpressure]] already requires.

- motivated_by:: [[decisions#Mailbox backend is chosen per address]] {weight: secondary}

Rejected: a priority queue keyed by deferral status. Status changes with the configuration, so every change would rebuild the heap.

### D9. Memory is released on deactivation

When an actor is deactivated, the node removes:
- its `mailboxes` entry, when the mailbox is empty (a store mailbox reopens from storage);
- its `outbox` entry, when nothing is owed;
- its timer keys, which already go when timers fire.

`NodeStats.activations` becomes a counter, with the per-address map behind a debug feature. Idle detection uses an ordered set of `(last_active, address)`, so `deactivate_idle` and `next_deadline` look at the oldest entries only. In the cluster:
- `virtuals` keeps only the actors active now;
- `PeerConn.pairs` evicts a pair after it has been idle for `pair_idle_ms`;
- replication `acked` drops the actors whose shard moved.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary, note: "state is recoverable, so memory need not be kept"}

### D10. Cluster transport: stream budget, a lock per peer, ordered windows

- **Stream limit.** quinn's `TransportConfig::max_concurrent_bidi_streams` is set explicitly (default 256). `max_pair_streams` is clamped below it, leaving a reserve for control streams. Past the cap, pairs share streams by hash, as [[decisions#QUIC with mutual TLS and a stream per actor pair]] already says.
- **Locks.** `peers` becomes a map of `Arc<PeerState>`, each with its own lock. Connecting and `open_bi` run outside every map lock, and a `OnceCell` per peer ensures one connection attempt.
- **Order.** Each pair stream has one sender task with a FIFO queue. It keeps up to `window` frames in flight (default 16) and reads replies in order, because QUIC streams are ordered. Retries resend from the head of that queue with exponential back-off and a per-message deadline. This replaces the task per message, so retries cannot reorder a pair.
- **View.** The membership view is kept as `ArcSwap<View>` with an atomic version. `refresh` compares versions before it takes an `Arc`, and no path clones the whole view.

- motivated_by:: [[constraints#QUIC limits streams per connection]] {weight: primary}
- motivated_by:: [[decisions#At-least-once inside the cluster with journal deduplication]] {weight: secondary}

### D11. Standby shipping in parallel, without pre-flight

`StandbyLink::replicate` becomes async. The owner ships one append to every standby concurrently and waits until the configured number have acknowledged. It skips the pre-flight round trip when a standby's acknowledged head equals the local head before the append, which is the common case. Shipping is batched together with group commit (D5).

- motivated_by:: [[decisions#Optional standby journal replication]] {weight: primary}
- motivated_by:: [[decisions#Unreplicated entries stay unreadable until the quorum catches up]] {weight: secondary}

### D12. Durability by origin (opt-in, last phase)

A message an actor sent is already in that actor's journal, and the outbox re-sends it until delivery. Storing it again in a durable mailbox duplicates the record. With the route option `durability: by_origin`:
- **From an actor.** An envelope whose message id is a recorded send IRI waits in memory.
- **From outside.** An envelope with no recorded sender (`NodeHandle::send`, a relay inbox, an HTTP ingress) is stored durably.
- **Confirmation.** The receiver confirms a message from an actor only after it has journaled the turn that consumed the message (`vt:consumed`, an in-process call or a cluster control frame). The sender keeps that send owed until then, and its outbox mark advances past it only after confirmation.

After a crash, the volatile part is rebuilt by the senders' outbox replay, and duplicates are dropped by D7. This is how reliable delivery works in Akka (producer-side buffer with consumer confirmation), applied to the journal Vertep already has.

- motivated_by:: [[decisions#Event-sourced actors]] {weight: primary}
- motivated_by:: [[constraints#Relays deliver at least once]] {weight: secondary}

The cost is one control message per actor message. Deferred messages keep their sender's mark from advancing, so a restarted sender re-sends more, and deduplication absorbs it. The default stays `durable`, so existing deployments behave as before.

## Constraints

- [[constraints#Tiramemsu has a single writer per file]] binds D3 and D5. One transaction per shard at a time is what makes checking the heads of a batch exact.
- [[constraints#QUIC limits streams per connection]] binds D10.
- [[constraints#Relays deliver at least once]] binds D7 and D12. Deduplication must stay exact.
- **New: "SQLite NORMAL synchronous can lose the newest commits".** In WAL mode with `synchronous=NORMAL`, a commit is durable only after the next checkpoint, so a power loss or OS crash can roll back the newest transactions (source: SQLite documentation, PRAGMA synchronous). It constrains D2 and [[openspec:event-journal#Transactional Outbox]].

## Risks / Trade-offs

- **Immediate durability adds a disk sync to every commit.** → Group commit spreads it across the batch. The benchmarks report both modes, and `Relaxed` stays available with its risk documented.
- **The volatile table is a new dependency on a tiramemsu API.** → Phase 2 starts by checking that `set_volatile`, `clear_volatile` and listing by subject work inside `Db::transact`. If not, a tiramemsu change goes upstream before the rest of D4.
- **A batch append makes one actor's commit wait for others on its shard.** → `max_wait_ms = 0` batches only work that is already ready, so no turn waits for a future one.
- **Shard groups change which thread a node runs on, and tests rely on one node.** → The default k = 1. Group tests run k = 4 on a virtual clock per group.
- **Durability by origin adds a protocol message.** → It is opt-in and in the last phase, and it can be dropped without affecting D1 to D11.
- **This branch stacks on unmerged `mailbox-backends`.** → Merge that first, or keep the cherry-picked commit as this branch's first commit.

## Migration Plan

- Existing tiramemsu files open unchanged. On first open by this version:
  - outbox marks and `vt:dispatched` marks in `<A>/outbox` graphs are read once, written as volatile rows, and the triples are left as history;
  - store-mailbox rows in `<A>/mailbox` graphs move the same way.
- The migration is idempotent and recorded in a volatile row `vt:bookkeeping-version = 1`.
- Rollback: an older version ignores the volatile rows. It would see stale outbox marks and re-dispatch, and receivers deduplicate.

## Open Questions

- **The freeze.** Should this be folded into `m13-production-readiness`, or recorded as an exception to [[decisions#New milestones wait for production readiness]]? This is the maintainer's choice.
- **`dedup_window` default.** 4096 ids per active actor is the working assumption, to be confirmed by the benchmarks.
- **Follow-ups:**
  - one heartbeat lease per node renewing all of its shards;
  - pre-copy hand-off (ship while serving, drain, ship the delta);
  - a cluster frame that does not wrap the envelope as JSON;
  - journal archival below the oldest kept snapshot.
