## 1. Crate setup

- [ ] 1.1 Create crate `vertep-cluster` in the workspace, depending on `vertep-runtime` and `vertep-store`, with cargo features `gossip`, `etcd`, `kubernetes` and `quic`
- [ ] 1.2 Add a crate-level `//! @tg: implements:: [[decisions#Shard-based placement]]` annotation on `vertep-cluster/src/lib.rs`
- [ ] 1.3 Define cluster metadata (cluster id, shard count, hash function version, replication factor) and its persistence
- [ ] 1.4 Add a placement-aware directory hook and a fencing-epoch field on journal appends to the runtime and store interfaces, behind the cluster feature, without changing M2 behaviour when it is off

## 2. Cluster membership

- [ ] 2.1 Implement the membership trait (view, subscribe, acquire, renew, release, revoke); annotate `@tg: implements:: [[openspec:cluster-membership#Membership Trait]]`; write a shared membership test kit run against every implementation, with tests for scenarios "Runtime runs unchanged on any implementation" and "Subscribers receive view changes in order" annotated `@tg: verifies:: [[openspec:cluster-membership#Membership Trait#<Scenario>]]`
- [ ] 2.2 Implement view versions and stale-view rejection; annotate `@tg: implements:: [[openspec:cluster-membership#Membership View Versions]]`; test "Newer view replaces older view" and "Stale view is ignored" with `@tg: verifies::` annotations
- [ ] 2.3 Implement join with cluster id, shard count and node id checks; annotate `@tg: implements:: [[openspec:cluster-membership#Graceful Join]]`; test "Compatible node joins", "Node with a different shard count is refused" and "Duplicate node identifier is refused" with `@tg: verifies::` annotations
- [ ] 2.4 Implement leave with drain (refuse activations, finish macrosteps, flush outboxes, hand over shards); annotate `@tg: implements:: [[openspec:cluster-membership#Graceful Leave With Drain]]`; test "Leaving node hands over its shards" and "Messages arriving during drain are forwarded" with `@tg: verifies::` annotations
- [ ] 2.5 Implement suspect and failed detection with configured times; annotate `@tg: implements:: [[openspec:cluster-membership#Failure Detection]]`; test "Crashed node is marked failed", "Suspect node that answers is restored" and "Leases of a failed node are not reassigned early" with `@tg: verifies::` annotations
- [ ] 2.6 Implement static membership; annotate `@tg: implements:: [[openspec:cluster-membership#Static Membership]]`; test "Members from configuration" and "Unlisted node cannot join" with `@tg: verifies::` annotations
- [ ] 2.7 Implement gossip membership on SWIM (foca) with lease granting by a deterministic leader; annotate `@tg: implements:: [[openspec:cluster-membership#Gossip Membership]]`; test "Cluster forms from one seed" and "Indirect probe avoids a false failure" with `@tg: verifies::` annotations
- [ ] 2.8 Implement etcd and Kubernetes Lease membership; annotate `@tg: implements:: [[openspec:cluster-membership#External Membership]]`; test "Leases follow the external service" and "Loss of the external service stops new leases" against containerised etcd and kind, with `@tg: verifies::` annotations
- [ ] 2.9 Implement shard leases with strictly increasing fencing epochs; annotate `@tg: implements:: [[openspec:cluster-membership#Shard Leases With Fencing Epochs]]`; test "New grant increases the epoch", "Renewal keeps the epoch" and "At most one valid lease per shard" with `@tg: verifies::` annotations
- [ ] 2.10 Implement lease expiry with safety margin on a monotonic clock, revocation and planned handover; annotate `@tg: implements:: [[openspec:cluster-membership#Lease Expiry And Handover]]`; test "Owner stops at expiry", "Planned handover" and "Old owner rejoins after a partition" with `@tg: verifies::` annotations

## 3. Shard placement

- [ ] 3.1 Enforce a power-of-two shard count recorded at creation; annotate `@tg: implements:: [[openspec:shard-placement#Fixed Shard Count]]`; test "Shard count recorded at creation" and "Non power of two is rejected" with `@tg: verifies::` annotations
- [ ] 3.2 Implement the versioned platform-independent IRI hash and shard computation; annotate `@tg: implements:: [[openspec:shard-placement#Shard Assignment By Actor IRI Hash]]`; test "Same IRI, same shard everywhere" (golden vectors checked in CI on Linux, macOS, x86_64 and aarch64) and "Hash function version is recorded" with `@tg: verifies::` annotations
- [ ] 3.3 Map each shard to its own journal store instance (one tiramemsu file per shard); annotate `@tg: implements:: [[openspec:shard-placement#One Journal File Per Shard]]`; test "Actors of one shard share a store" and "Shards write independently" with `@tg: verifies::` annotations
- [ ] 3.4 Gate activation on holding an unexpired lease; annotate `@tg: implements:: [[openspec:shard-placement#Activation On Shard Owner Only]]`; test "Owner activates on first message" and "Non-owner refuses activation" with `@tg: verifies::` annotations
- [ ] 3.5 Implement the single-activation path end to end; annotate `@tg: implements:: [[openspec:shard-placement#Single Activation Guarantee]]`; test "Concurrent activation attempts" with a `@tg: verifies::` annotation (the fault-injection scenario is covered in group 6)
- [ ] 3.6 Carry the fencing epoch on every append and outbound frame, reject lower epochs in journal stores and replication receivers, and drop fenced frames into the dead-letter graph; annotate `@tg: implements:: [[openspec:shard-placement#Journal Fencing Backstop]]`; test "Stale owner append fails after split brain" and "Fenced outbound messages are dropped" with `@tg: verifies::` annotations
- [ ] 3.7 Implement forwarding to the owner with a hop limit; annotate `@tg: implements:: [[openspec:shard-placement#Message Forwarding To Shard Owner]]`; test "Message sent to the wrong node" and "Hop limit reached" with `@tg: verifies::` annotations
- [ ] 3.8 Implement the rebalancer and journal file shipping with suffix resume and end-version verification; annotate `@tg: implements:: [[openspec:shard-placement#Rebalancing By Journal File Shipping]]`; test "Shard moved to a new node" and "Shipping interrupted" with `@tg: verifies::` annotations
- [ ] 3.9 Implement standby replication with quorum acknowledgement and failover; annotate `@tg: implements:: [[openspec:shard-placement#Journal Replication To Standbys]]`; test "Failover to a standby" and "No standby with replication factor one" with `@tg: verifies::` annotations
- [ ] 3.10 Implement lease-only moves against a shared journal store (in-memory shared store for tests); annotate `@tg: implements:: [[openspec:shard-placement#Rebalancing By Lease Transfer]]`; test "Shard moved with a shared journal" with a `@tg: verifies::` annotation
- [ ] 3.11 Implement the offline resharding tool (`vt cluster reshard`) that doubles N, and document the procedure; annotate `@tg: implements:: [[openspec:shard-placement#Resharding Procedure]]`; test "Doubling the shard count" and "Resharding refuses a running cluster" with `@tg: verifies::` annotations

## 4. Cluster transport

- [ ] 4.1 Implement the QUIC transport and register it as a runtime transport and Event I/O Processor; annotate `@tg: implements:: [[openspec:cluster-transport#QUIC Cluster Transport]]`; test "Send reaches an actor on another node" and "Event origin names the sender" with `@tg: verifies::` annotations
- [ ] 4.2 Implement mutual TLS with cluster-issued certificates and membership checks; annotate `@tg: implements:: [[openspec:cluster-transport#Mutual TLS Within The Cluster]]`; test "Member connects", "Foreign certificate refused" and "Removed member refused" with `@tg: verifies::` annotations
- [ ] 4.3 Implement the frame codec with a maximum frame size; annotate `@tg: implements:: [[openspec:cluster-transport#Message Framing]]`; test "Frame decodes to the same event" (with a property-based round trip) and "Oversized frame rejected" with `@tg: verifies::` annotations
- [ ] 4.4 Implement per-pair streams with a stream cap and sequenced multiplexing fallback; annotate `@tg: implements:: [[openspec:cluster-transport#Ordering Per Sender And Receiver]]`; test "Two sends arrive in order" and "Independent pairs do not block each other" with `@tg: verifies::` annotations
- [ ] 4.5 Couple mailbox bounds to QUIC flow control; annotate `@tg: implements:: [[openspec:cluster-transport#Flow Control And Backpressure]]`; test "Full mailbox slows the sender" and "Backpressure is per mailbox" with `@tg: verifies::` annotations
- [ ] 4.6 Implement retries until stored acknowledgement, delivery deadlines, and receiver-side deduplication reusing journal dedup; annotate `@tg: implements:: [[openspec:cluster-transport#At-Least-Once Delivery With Deduplication]]`; test "Lost acknowledgement causes a duplicate that is discarded" and "Retries stop at the deadline" with `@tg: verifies::` annotations
- [ ] 4.7 Implement the routing table fed by membership views and lease epochs; annotate `@tg: implements:: [[openspec:cluster-transport#Routing Via The Shard Table]]`; test "Message routed to the owner" and "Routing follows a handover" with `@tg: verifies::` annotations
- [ ] 4.8 Implement redirects and epoch-checked routing updates; annotate `@tg: implements:: [[openspec:cluster-transport#Redirect On Stale Routing]]`; test "Sender learns the new owner" and "Redirect with an older epoch is ignored" with `@tg: verifies::` annotations

## 5. Worker pools

- [ ] 5.1 Implement on-demand stateless worker activations; annotate `@tg: implements:: [[openspec:worker-pools#Stateless Worker Activations]]`; test "Activations created on demand" and "Any activation can serve a request" with `@tg: verifies::` annotations
- [ ] 5.2 Implement worker-type declarations per node and pool placement; annotate `@tg: implements:: [[openspec:worker-pools#Pool Placement Across Nodes]]`; test "Workers spread across nodes" and "Node without the worker type is not used" with `@tg: verifies::` annotations
- [ ] 5.3 Implement round robin, least outstanding and prefer local policies; annotate `@tg: implements:: [[openspec:worker-pools#Load Balancing Policies]]`; test "Least outstanding requests" and "Prefer local node" with `@tg: verifies::` annotations
- [ ] 5.4 Implement per-node activation limits, per-activation request limits and a bounded pool queue; annotate `@tg: implements:: [[openspec:worker-pools#Worker Concurrency Limits]]`; test "Limit reached" and "Queue bound reached" with `@tg: verifies::` annotations
- [ ] 5.5 Ensure workers hold no journal or durable state; annotate `@tg: implements:: [[openspec:worker-pools#No Journal For Workers]]`; test "Activation discarded under memory pressure" and "Worker writes nothing to the journal store" with `@tg: verifies::` annotations
- [ ] 5.6 Deliver worker results to callers as journaled input events matched by correlation id; annotate `@tg: implements:: [[openspec:worker-pools#Results Journaled By The Caller]]`; test "Replay does not call the worker" and "Duplicate result discarded" with `@tg: verifies::` annotations
- [ ] 5.7 Report worker failures as error events or request timeouts, with a single retry for idempotent worker types; annotate `@tg: implements:: [[openspec:worker-pools#Worker Failure Reporting]]`; test "Worker node crashes" and "Idempotent worker is retried" with `@tg: verifies::` annotations

## 6. Fault injection and verification

- [ ] 6.1 Build a Jepsen-style harness that runs a multi-node cluster in-process and in containers, injecting network partitions, one-way packet loss, process pauses, crashes, restarts and clock skew beyond the safety margin
- [ ] 6.2 Write a history checker that verifies every actor journal is linear, no journal version holds two entries, every acknowledged message was processed exactly once, and no fenced side effect was accepted; annotate the fault-injection test `@tg: verifies:: [[openspec:shard-placement#Single Activation Guarantee#Fault injection never yields two writers]] {level: e2e}`
- [ ] 6.3 Run the harness against each membership implementation (static, gossip, etcd, Kubernetes) and against replication factors 1 and 3 in nightly CI
- [ ] 6.4 In `vertep-lean`, model shard leases, epochs, fenced appends and replication receivers, and prove that for every shard and epoch at most one writer's appends are accepted and that accepted journal versions are totally ordered across epochs; no `sorry`, axioms pinned in tests
- [ ] 6.5 Use the Lean lease and fencing model as a differential oracle: replay histories recorded by the fault-injection harness through it and require identical accept and reject decisions

## 7. Documentation and graph

- [ ] 7.1 Record the design decisions D1 to D9 in `tg.md/decisions.md` as typed `Decision` sections with `motivated_by::` edges to [[decisions#Shard-based placement]], [[constraints#Tiramemsu has a single writer per file]] and [[constraints#Relays deliver at least once]] as stated in design.md
- [ ] 7.2 Add constraints that bind this area to `tg.md/constraints.md` (for example lease safety under clock skew and QUIC per-connection limits) with `constrains:: [[openspec:<capability>#<Requirement>]] {strength: must}` edges
- [ ] 7.3 Update [[capabilities#Cluster membership]], [[capabilities#Shard placement]], [[capabilities#Cluster transport]] and [[capabilities#Worker pools]] with `specified_by::` edges to their requirements, and keep the four capabilities in that file's `openspec:` frontmatter
- [ ] 7.4 Update [[components#vertep-cluster]] with `provides::` edges to the requirements it implements and its status
- [ ] 7.5 Write the cluster architecture view in `tg.md/` with Mermaid diagrams for topology, lease handover with fencing, routing and rebalancing, typed `View` with `describes::` edges
- [ ] 7.6 Run `tg check`, `tg ontology check` and `tg trace cluster-membership shard-placement cluster-transport worker-pools --gaps`, and close every gap they report
