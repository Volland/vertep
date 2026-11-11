## Why

After M0 and M1 Vertep can interpret and author typed statecharts, but a chart only runs inside a test harness: nothing receives messages, survives a crash, keeps history or stores state in the graph. M2 turns a chart into a durable actor on one node, which is the base every later milestone (agents, cluster, federation, WASM) builds on.

## What Changes

- Add the `Actor` trait and IRI addresses, and the three lifecycle classes: virtual actors (activated on message, deactivated when idle, never invoked), invoked actors (owned by the invoking state) and stateless workers (pooled, no journal), with one turn equal to one SCXML macrostep and `vt:restart` supervision.
- Add pluggable `Mailbox` and `Transport` traits, the local transport as an SCXML Event I/O Processor, a directory that resolves an address to a mailbox, mailboxes that outlive activations and trigger them, `vt:request` sugar, `vt:defer`, and the unhandled-event policy with a queryable dead-letter graph.
- Add the event journal: input events with message ids, the chart version, outbox sends and timers per entry, compare-and-set append, deduplication, transactional outbox dispatch, snapshots and deterministic replay, with external and LLM results recorded as inputs.
- Add the storage contracts `JournalStore` and `GraphStore` with a shared test kit, the tiramemsu backend implementing both in one file per shard, an Oxigraph and SPARQL `GraphStore` adapter, an RDF 1.2 capability flag with an RDF 1.1 reification fallback, and current-state projections.
- Add durable timers: delayed sends and reminders journaled, cancellable, fired as input events, and reactivating virtual actors.
- Add chart versioning: version IRIs recorded per journal segment, replay with the recorded version, pin-until-final, explicit migrations recorded as journal entries, a compatibility check, and forking from before a migration.
- Add the `vt:` runtime ontology module with SHACL shapes, PROV-O alignment by subclassing, and RDF 1.2 triple-term annotations.

## Capabilities

### New Capabilities

- `actor-model`: the `Actor` trait, IRI addresses, the virtual, invoked and stateless-worker lifecycle classes, turns as macrosteps, and supervision.
- `messaging`: mailboxes, transports, address resolution, `vt:request`, `vt:defer`, the unhandled-event policy and dead letters.
- `event-journal`: event-sourced actor state with compare-and-set append, deduplication, transactional outbox, snapshots and deterministic replay.
- `storage`: the `JournalStore` and `GraphStore` contracts, the tiramemsu backend, the Oxigraph and SPARQL adapter, the RDF 1.1 fallback and projections.
- `durable-timers`: journaled delayed sends and reminders that fire as input events and reactivate actors.
- `chart-versioning`: version pinning per journal segment, pin-until-final, migrations as journal entries, compatibility checks and forks.
- `runtime-ontology`: the `vt:` runtime ontology module, its SHACL shapes, PROV-O alignment and triple-term annotations.

### Modified Capabilities

None. The M0 and M1 capabilities this change builds on (`chart-model`, `scxml-interpretation`, `datamodel`, `chart-rdf`, `type-system`, `formal-model`) keep their requirements; the runtime consumes them.

## Impact

- **Crates:** new `vertep-runtime`, `vertep-store`, `vertep-tiramemsu`, `vertep-oxigraph` and the `vertep` facade; `vertep-rdf` gains the runtime ontology module. See [[components#vertep-runtime]], [[components#vertep-store]], [[components#vertep-tiramemsu]], [[components#vertep-oxigraph]], [[components#vertep-rdf]] and [[components#vertep]].
- **Capability sections:** [[capabilities#Actor model]], [[capabilities#Messaging]], [[capabilities#Event journal]], [[capabilities#Storage]], [[capabilities#Durable timers]], [[capabilities#Chart versioning]] and [[capabilities#Runtime ontology]]; milestone [[roadmap#M2 Actor runtime]].
- **Binding decisions:** [[decisions#SCXML-native actors with vt extensions]], [[decisions#IRI identity and lifecycle classes]], [[decisions#Identity, mailbox and transport layers]], [[decisions#Event-sourced actors]], [[decisions#Two storage traits with tiramemsu as default]], [[decisions#Content-addressed chart versions]], [[decisions#Modular vt ontology on RDF 1.2]], [[decisions#No-std core and tokio shell]] and [[decisions#Non-blocking request and deferred events]].
- **Binding constraints:** [[constraints#Relays deliver at least once]], [[constraints#SPARQL Update has no compare-and-set]], [[constraints#Tiramemsu has a single writer per file]], [[constraints#SCXML discards unmatched events]] and [[constraints#RDF is open-world]].
- **Dependencies:** `tokio`, `tiramemsu` (crates.io), `oxigraph`, an RDFC-1.0 canonicalizer and a SHACL validator; the Lean package `vertep-lean` gains the journal protocol theorems.
- **Out of scope:** multi-node placement (M4), DID identities and relays (M5), deciders and LLM actors (M3); the trait boundaries are drawn so those land without changing these requirements.
