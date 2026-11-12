---
type: Constraint
source: "External specifications, protocols and libraries; each section names its origin"
---
# Constraints

Rules Vertep did not choose: what the SCXML standard, RDF, SPARQL, the delivery path, the default store and the proof tooling impose on the design.

## SCXML interpretation algorithm

The W3C SCXML Recommendation (2015) defines a normative step-by-step interpretation algorithm and a conformance suite of about 200 tests.

- constrains:: [[decisions#SCXML semantics are normative]] {strength: must, source: "W3C SCXML Recommendation, Appendix D"}
- constrains:: [[decisions#Full SCXML first]] {strength: must, source: "W3C SCXML IRP tests"}

Most conformance tests assume the `ecmascript` datamodel, so passing the suite needs an ECMAScript datamodel.

## SCXML discards unmatched events

In SCXML an external event that enables no transition is dropped silently, which is safe for UI machines and unsafe for durable actors.

- constrains:: [[decisions#Non-blocking request and deferred events]] {strength: must, source: "W3C SCXML 3.13 and Appendix D"}

## RDF is open-world

A missing triple in RDF does not mean a fact is false, while execution needs closed facts such as exactly one initial state.

- constrains:: [[decisions#IR is the hub]] {strength: must, source: "RDF 1.2 Semantics"}
- constrains:: [[decisions#Modular vt ontology on RDF 1.2]] {strength: must, source: "RDF 1.2 Semantics; SHACL"}

## Proc macros run at compile time

A Rust procedural macro runs inside the compiler and must emit Rust types; it cannot read a triple store or go through RDF.

- constrains:: [[decisions#IR is the hub]] {strength: must, source: "Rust proc_macro"}
- constrains:: [[decisions#One IR type language]] {strength: should, source: "Rust proc_macro"}

## SPARQL Update has no compare-and-set

SPARQL 1.1 Update offers no conditional append, and transaction support varies between stores, so a generic endpoint cannot promise linearizable appends.

- constrains:: [[decisions#Two storage traits with tiramemsu as default]] {strength: must, source: "SPARQL 1.1 Update"}

## Relays deliver at least once

Store-and-forward mailboxes such as DIDComm mediators and TSP intermediaries redeliver after crashes and timeouts, so exactly-once delivery is impossible.

- constrains:: [[decisions#Event-sourced actors]] {strength: must, source: "DIDComm v2 mediators; ToIP TSP"}
- constrains:: [[decisions#Identity, mailbox and transport layers]] {strength: must, source: "DIDComm v2 mediators; ToIP TSP"}
- constrains:: [[decisions#DIDComm mailboxes acknowledge after the append]] {strength: must, source: "DIDComm v2 mediators; ToIP TSP"}
- constrains:: [[decisions#Relay outbox owns cross-trust retries]] {strength: must, source: "DIDComm v2 mediators; ToIP TSP", note: "a re-send keeps its message id"}

## OPFS sync access handles need a dedicated worker

The Origin Private File System's synchronous access handles, which tiramemsu's OPFS VFS needs, exist only in dedicated workers, and each handle is exclusive to one holder.

- constrains:: [[decisions#One worker hosts the core, the shell and the journal]] {strength: must, source: "WHATWG File System Standard; sqlite-wasm-vfs opfs-sahpool"}
- constrains:: [[decisions#Browser storage is explicit, exclusive and native-format]] {strength: must, source: "WHATWG File System Standard"}

Neither the page's main thread nor a SharedWorker can open the store, and a second tab must be refused rather than wait. Requirements `Browser Journal Storage` and `Worker-Hosted Actor System` of `wasm-bindings` follow from it.

## Tiramemsu has a single writer per file

Tiramemsu serializes every transaction through one writer per database file and has no network server.

- constrains:: [[decisions#Two storage traits with tiramemsu as default]] {strength: must, source: "tiramemsu lat.md/architecture.md#Connections and Concurrency"}
- constrains:: [[decisions#Shard-based placement]] {strength: must, source: "tiramemsu lat.md/constraints.md#Single Writer"}

The single writer is what makes a compare-and-set journal append atomic, and also why one file must hold one shard rather than a whole node.

## Aeneas supports a Rust subset

Aeneas translates only a restricted subset of safe Rust to Lean, with limited traits and no async.

- constrains:: [[decisions#Lean verification bridge]] {strength: should, source: "Aeneas"}
- constrains:: [[decisions#No-std core and tokio shell]] {strength: should, source: "Aeneas"}

## W3C serves a bot challenge to scripted clients

The W3C web server answers bursts of scripted requests with HTTP 429 and a Cloudflare challenge, so the IRP files cannot be fetched reliably by a script or in CI.

- constrains:: [[decisions#IRP suite from a pinned mirror]] {strength: must, source: "observed 2026-11-11 on www.w3.org/Voice/2013/scxml-irp"}

## The IRP stylesheet is XSLT 2.0

`confEcma.xsl`, which turns IRP `.txml` templates into ECMAScript charts, uses XSLT 2.0 features such as `xsl:analyze-string`, so libxslt's `xsltproc` cannot run it.

- constrains:: [[decisions#IRP suite from a pinned mirror]] {strength: must, source: "confEcma.xsl version=\"2.0\""}

## Tiramemsu cannot hold unasserted triple terms or reifier IRIs

Every tiramemsu statement id is an asserted statement: a triple term in inserted data is asserted, reifiers resolve to statement ids and a reifier IRI is refused.

- constrains:: [[decisions#Two storage traits with tiramemsu as default]] {strength: should, source: "tiramemsu lat.md/query.md SPARQL 1.2 deviation"}

The backend rejects triple terms in entries and state deltas and writes reifiers as annotation syntax; reifier identity is not preserved.

## Tiramemsu's default graph is the union of all graphs

A tiramemsu query without `GRAPH` also sees entry graphs, state graphs and bookkeeping, so portable queries scope with `GRAPH`; the backend declares `union_default_graph`.

- constrains:: [[decisions#Two storage traits with tiramemsu as default]] {strength: should, source: "tiramemsu D28"}

## Tiramemsu canonicalises literals

Tiramemsu stores literals in canonical form, for example `"08"^^xsd:integer` reads back as `"8"`, so entries with non-canonical lexical forms do not round-trip byte for byte.

- constrains:: [[decisions#Journal entries are stamped by the store]] {strength: should, source: "tiramemsu"}

## Typed charts fix the error payload

The type checker gives every `error.*` event the payload `{ kind: string, message: string }` unless the chart declares that event by name.

- constrains:: [[decisions#Agent error events extend the error payload]] {strength: must, source: "vertep-core types/events.rs system_event_type"}

## The type language has no string enumerations

The IR type language spells an enumeration as a union of tags, which its JSON Schema projection turns into objects with a constant `tag`.

- constrains:: [[decisions#Enumerations are tagged unions in structured output]] {strength: must, source: "vertep-core types/syntax.rs and the vertep-rdf JSON Schema projection"}

## The SHACL validator bounds only integers

The `vertep-rdf` SHACL Core validator supports `sh:minInclusive` and its siblings on integers only, so decimal ranges cannot be shapes.

- constrains:: [[decisions#The agent ontology ships with vertep-rdf]] {strength: should, source: "vertep-rdf shacl.rs"}

## Clock skew beyond the safety margin breaks lease exclusivity

A lease holder judges expiry by its own clock; a pause or skew longer than the safety margin lets a stale owner believe it still holds a lease the authority has granted again.

- constrains:: [[decisions#Leases with fencing epochs from the membership authority]] {strength: must, source: "distributed systems: leases without synchronized clocks (Gray and Cheriton 1989)"}
- constrains:: [[decisions#Fencing on appends and outbound frames]] {strength: must, source: "distributed systems: leases without synchronized clocks (Gray and Cheriton 1989)", note: "fencing, not timing, carries safety"}
- constrains:: [[crates/vertep-cluster/src/lease.rs#HeldLeases]] {strength: must, source: "Gray and Cheriton 1989"}

Safety therefore never depends on clocks; availability does.

## QUIC limits streams per connection

A QUIC peer advertises a maximum number of concurrent bidirectional streams per connection, so one stream per actor pair cannot grow without bound.

- constrains:: [[decisions#QUIC with mutual TLS and a stream per actor pair]] {strength: must, source: "RFC 9000 section 4.6, MAX_STREAMS"}
- constrains:: [[crates/vertep-cluster/src/transport.rs#TransportConfig]] {strength: must, source: "RFC 9000 section 4.6"}

Past the cap, pairs share streams and stay ordered per pair.

## Kubernetes stores leases but does not expire them

A `coordination.k8s.io/v1` `Lease` records a holder, a duration and a renew time, but the API server never expires it; clients decide when a lease has lapsed.

- constrains:: [[decisions#Kubernetes grants expire by observation]] {strength: must, source: "Kubernetes API reference, Lease v1 coordination.k8s.io"}
- constrains:: [[crates/vertep-cluster/src/external/kubernetes.rs#KubeAuthority]] {strength: must, source: "Kubernetes Lease API"}

Comparing `renewTime` with a local wall clock would make lease safety depend on clock synchronisation.

## etcd lease TTLs are whole seconds

etcd grants leases with a TTL in whole seconds, at least about one and a half election timeouts, and expires them on its own sweep.

- constrains:: [[decisions#etcd expires shard grants and a permanent key holds each shard's epoch]] {strength: must, source: "etcd v3 lease API, minimum TTL from the election timeout"}
- constrains:: [[crates/vertep-cluster/src/external/etcd.rs#EtcdAuthority]] {strength: must, source: "etcd v3 lease API"}

Lease terms and detection times are rounded up to whole seconds, and tests run etcd with a short election timeout.

## The Docker Desktop kernel has no netem

The linuxkit kernel of Docker Desktop has neither the `netem` queueing discipline nor the iptables `statistic` match, so a container cannot drop a share of its packets.

- constrains:: [[decisions#Container fault injection runs every node as a process]] {strength: should, source: "Docker Desktop linuxkit kernel (tc: RTNETLINK not supported)"}

CI runners on Linux have netem; the harness uses it where it works.

