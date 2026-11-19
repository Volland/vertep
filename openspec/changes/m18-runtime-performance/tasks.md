Each phase lands as its own pull request with green CI. Phase 0 comes first, so every later phase is measured against it.

## 0. Measure (D1)

- [x] 0.1 `CountingStorage` in the `vertep-store` test kit: counts commits, quads written and read, SPARQL queries, and journal `read` and `head` calls, per call site
- [x] 0.2 `benches/runtime` (criterion), one suite that is also `production-readiness#Benchmark Suite`, moved here from M13 ([[decisions#Benchmarks measure the claims]]): pure `step` throughput on three chart sizes; journal append latency and replay speed per backend (memory, tiramemsu file); decider round trip with a scripted decider; turns/s (memory and tiramemsu, with and without a send); activation at 10, 1k and 100k entries (with and without a snapshot); receive with 0, 100 and 10k deferred messages; a cross-node send round trip on loopback QUIC
- [ ] 0.2a Size and cold-start measurements for `@vertep/web` and the Python wheel (2026-11-19: `scripts/measure-sizes.sh` reads built artifacts and `bench-gate.py collect --sizes` records them; still to wire into the `wasm` and `python` CI jobs, where the artifacts are built)
- [x] 0.3 Record the baseline on `main` (before any change below) in `benches/baseline.json` with commit, machine and durability mode; it doubles as the last-release results the gate compares against
- [x] 0.4 Budget tests through `CountingStorage`, marked `#[ignore]` until the phase that makes each one pass: a turn commits once, activation reads only after the snapshot, a duplicate is dropped without a full read
- [x] 0.5 Regression gate job in CI against the last release's recorded results, failing over a stated threshold (the `bench-gate` job runs the suite at the last release tag and at the change on one runner; passes with a note until a release contains the suite)
- [x] 0.5a Results page under `site/` with machine and commit; later phases add their before and after numbers to it
- [x] 0.6 Annotate: `@tg: implements:: [[openspec:production-readiness#Benchmark Suite]]` on the harness, the size measurements and the gate, and `verifies::` for `#Results are recorded` and `#A regression fails CI`; `@tg: implements:: [[openspec:runtime-performance#Hot Path Benchmarks]]` and `[[openspec:runtime-performance#Operation Budgets]]` on the harness and the wrapper; `@tg: implements:: [[openspec:runtime-performance#Benchmark Regression Gate]]` on the gate script; `@tg: verifies::` on each budget test for its scenario

## 1. Correctness under load (D2, D10 in part)

- [ ] 1.1 `vertep-cluster` transport: set quinn's `max_concurrent_bidi_streams` explicitly (default 256) and clamp `max_pair_streams` below it, leaving a reserve
- [ ] 1.2 Replace the transport-wide `peers` mutex with a map of `Arc<PeerState>`, each with its own lock and a `OnceCell` connection; no lock is held across `connect` or `open_bi`
- [ ] 1.3 One sender task per pair stream with a FIFO queue, exponential back-off and per-message deadline; remove the task per message (`runtime.rs` retry loop)
- [ ] 1.4 Test: 1,000 pairs to one peer with a limit of 256 deliver everything, each pair in order (`Stream Budget Within The Peer Limit#More pairs than the limit`)
- [ ] 1.5 Test: a hanging handshake to peer P does not delay sends to peer Q (`#Slow peer does not stall others`)
- [ ] 1.6 Test: retried e1 still precedes e2 (`Ordering Per Sender And Receiver#Retry keeps the order`)
- [ ] 1.7 `vertep-tiramemsu`: open journal shards with `Durability::Immediate` by default; add `JournalCapabilities::durable_commit`; `NodeConfig::durability` with `Relaxed` opt-in and a startup log line naming the risk
- [ ] 1.8 Test: default open declares durable commits; relaxed open declares not durable (`Durable Commit` scenarios)
- [ ] 1.9 Crash test with the testkit crash points: an entry whose sends were dispatched is present after restart (`Transactional Outbox#No send outlives its entry`)
- [ ] 1.10 Annotate: `implements::` on the stream budget, the per-peer state and the pair sender for `cluster-transport#Stream Budget Within The Peer Limit` and `#Ordering Per Sender And Receiver`; `implements::` on the shard open for `storage#Durable Commit` and on dispatch for `event-journal#Transactional Outbox`; `verifies::` above each test from 1.4 to 1.9

## 2. One commit per turn, unversioned bookkeeping, reads from the snapshot (D3, D4, D7)

- [ ] 2.1 Spike: confirm that tiramemsu 0.12 `Tx::set_volatile`, `clear_volatile` and listing by subject work inside `Db::transact` and roll back with it; if not, prepare the upstream tiramemsu change first
- [ ] 2.2 `AppendMeta`: `annotations`, `outbox_through`, `ack`, `snapshot`; `JournalCapabilities` flags for each; tiramemsu writes them in the append transaction
- [ ] 2.3 Node: pass annotations, outbox mark, acknowledgement and snapshot through the append when the store supports them, and fall back to separate writes otherwise
- [ ] 2.4 Volatile rows in tiramemsu for mailbox rows, outbox marks, `vt:dispatched` marks, heads, accepted message ids and snapshots; store the envelope once as compact JSON, with no duplicate `vt:payload`
- [ ] 2.5 `StoreMailbox` on volatile rows when its storage supports them, keeping the quad layout for other `Storage` implementations
- [ ] 2.6 One-time migration of `<A>/outbox` and `<A>/mailbox` triples to volatile rows, recorded as `vt:bookkeeping-version = 1`, idempotent
- [ ] 2.7 `JournalStore::latest_snapshot` and `contains_message` with default implementations and indexed tiramemsu implementations; measure the existing `read_range` (from M13 journal shipping) on tiramemsu against `Journal Range Read` and index it if it scans
- [ ] 2.8 `Node::load` reads the latest valid snapshot and `read_range` after it; outbox replay reads after the mark; `snapshot_keep` (default 2) prunes older snapshots
- [ ] 2.9 Deduplication: a bounded window (`dedup_window`, default 4096) and `contains_message`; drop the full `seen` set
- [ ] 2.10 Shared backend test kit: range read, message-id lookup, and one-commit append with annotations, mark and acknowledgement, run on memory and tiramemsu
- [ ] 2.11 Tests for the scenarios of `storage#One Commit Per Turn`, `#Unversioned Bookkeeping`, `#Journal Range Read`, `#Message Id Lookup`, `event-journal#Activation Reads From The Snapshot`, `#Snapshot Retention` and `#Indexed Deduplication`; un-ignore the budget tests from 0.4
- [ ] 2.12 Annotate the implementing items with `implements::` for each requirement above, and each test with `verifies::` for its scenario `{level: unit|integration}`

## 3. Mailbox internals and memory (D8, D9)

- [ ] 3.1 `MemoryMailbox` and `StoreMailbox` row cache: `ready` queue, deferred `stash`, `in_flight` slot or map, id index; `Mailbox::redefer` called by the node when the deferring descriptors change
- [ ] 3.2 `has_deliverable` answers whether any message is deliverable without counting; `len` without cloning envelopes
- [ ] 3.3 `capacity` and `overflow: reject | wait` in `RoutedMailboxes` and `MailboxConfig`; `MailboxError::Full`, `DeliveryError::Full`; outbox treats full as a refused delivery; `vt mailbox check` validates the new fields
- [ ] 3.4 Release on deactivation: empty mailbox handles, settled outbox records, dedup windows; `NodeStats.activations` as a total with the per-address map behind a debug feature
- [ ] 3.5 Idle set ordered by `(last_active, address)` for `deactivate_idle` and `next_deadline`
- [ ] 3.6 Cluster: `virtuals` holds only active actors; `PeerConn.pairs` evicts pairs idle past `pair_idle_ms`; replication `acked` drops moved actors
- [ ] 3.7 Tests for `messaging#Receive Cost Under Deferral`, `#Bounded Mailboxes`, `actor-model#Deactivation Releases Memory` and `#Idle Detection Without Scanning`; existing `Deferred Events` tests stay green
- [ ] 3.8 Annotate implementing items and tests (`implements::`, `verifies::`)

## 4. Group commit and shard groups (D5, D6)

- [ ] 4.1 `JournalStore::append_batch` with one result per actor; default implementation loops `append`; tiramemsu checks every head and fence in one transaction and syncs once
- [ ] 4.2 Node: gather a round's turns by shard, bounded by `group_commit.max_turns` and `max_wait_ms`; acknowledge and dispatch after the batch commits
- [ ] 4.3 `NodeGroup`: k nodes on k threads, each owning shards `s % k == i`, with a cross-group inbox (a `Send` channel and `Notify`) and a `NodeGroupHandle` that routes `send` and `ask`
- [ ] 4.4 Test kit: batch with one conflicting actor; no dispatch before commit
- [ ] 4.5 Tests for `storage#Group Commit Per Shard` and `actor-model#Shard Groups` (two shards on two threads, cross-group send, a group of one)
- [ ] 4.6 Benchmarks: turns/s at k = 1, 2, 4, 8 with immediate durability; record against the baseline
- [ ] 4.7 Annotate implementing items and tests

## 5. Cluster throughput (D10, D11)

- [ ] 5.1 Pair stream window (default 16): write up to the window, read replies in order, resend from the first unacknowledged frame
- [ ] 5.2 Membership view as `ArcSwap<View>` with an atomic version; `refresh` compares versions first; remove whole-view clones from route, receive, activation gate and `QuicWire::send`
- [ ] 5.3 `StandbyLink::replicate` async; ship to standbys concurrently; skip the pre-flight when the standby's acknowledged head equals the local head; ship group-commit batches
- [ ] 5.4 Kept-outbound queue: raw bytes instead of hex, writes off the async thread, notify-based wait instead of 2 ms polling
- [ ] 5.5 Tests for `cluster-transport#Pipelined Pair Streams` and `shard-placement#Parallel Standby Shipping` (with delayed `LocalStandby` links)
- [ ] 5.6 Annotate implementing items and tests

## 6. Durability by origin (D12, opt-in)

- [ ] 6.1 Route option `durability: durable | by_origin` (default `durable`)
- [ ] 6.2 Classify envelopes: a recorded send IRI goes to memory; no recorded sender goes to durable storage
- [ ] 6.3 Consumption confirmation: an in-process call within a node or group, and a cluster control frame between nodes; the sender keeps the send owed until confirmed, and the outbox mark advances only past confirmed sends
- [ ] 6.4 Crash tests: receiver restart loses nothing and consumes each message once; an outside message survives; a deferred message stays owed
- [ ] 6.5 Benchmark by-origin against durable mailboxes; record commits per message from `CountingStorage`
- [ ] 6.6 Annotate `implements:: [[openspec:messaging#Durability By Origin]]` and the scenario tests

## 7. Soak and docs

- [ ] 7.1 Soak test (`#[ignore]`, run nightly): one million messages to 100k actors with deactivation churn; assert `Bounded Long-Run Growth` scenarios with `CountingStorage` and node introspection
- [x] 7.2 Record decisions D1 to D12 in `tg.md/decisions.md` as `Decision` sections with `motivated_by::` and `supersedes::` edges (done 2026-11-16, marked proposed; the two superseded decisions point at their replacements)
- [ ] 7.2a As each phase lands, change its decision's text from proposed to accepted with the date, and update the superseded decisions to say so
- [x] 7.3 Add the constraint `SQLite NORMAL synchronous can lose the newest commits` to `tg.md/constraints.md` with `constrains::` edges to D2 and `[[openspec:event-journal#Transactional Outbox]]` `{strength: must}`
- [x] 7.4a `tg.md/views/performance.md` records the review and the plan; `views/persistence.md` no longer claims the append acknowledges the message (done 2026-11-16)
- [ ] 7.4 Update `tg.md/views/persistence.md` (Tiramemsu mapping once D3 and D4 land, storage traits), `views/runtime.md` (mailboxes, scheduling, shard groups) and `views/distribution.md` (routing over QUIC, replication), and list `runtime-performance` in the `openspec:` frontmatter of the view that explains it
- [ ] 7.5 `tg.md/roadmap.md`: section `M18 Runtime performance` (drafted 2026-11-16); `capabilities.md` `Runtime performance` (drafted 2026-11-16); `components.md`: `provides::` edges for the new requirements on `vertep-runtime`, `vertep-store`, `vertep-tiramemsu`, `vertep-cluster`
- [ ] 7.6 `CHANGELOG.md`, `site/docs/runtime.html` (tuning: durability, group commit, shard groups, capacity), results page
- [ ] 7.7 Run `tg check`, `tg ontology check` and `tg trace runtime-performance --gaps` (and for `storage`, `event-journal`, `messaging`, `actor-model`, `cluster-transport`, `shard-placement`) and close every gap
