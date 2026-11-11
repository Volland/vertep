# storage Specification

## Purpose
Defines the two storage contracts the runtime depends on, a journal store with linearizable per-actor compare-and-set appends and an RDF graph store for queries, and the backends that implement them, with tiramemsu as the default and Oxigraph or any SPARQL endpoint as graph stores.

## Requirements

### Requirement: Journal Store Contract
A journal store SHALL provide, per actor address, an append operation taking an expected position and a list of entries encoded as RDF datasets, and a read operation from a position; appends for one actor SHALL be linearizable and all-or-nothing.

#### Scenario: Atomic multi-entry append
- **WHEN** an append of three entries at expected position 10 succeeds
- **THEN** a later read from position 11 returns exactly those three entries in order

#### Scenario: Failed append writes nothing
- **WHEN** an append fails because of a position conflict or a storage error
- **THEN** a later read shows none of its entries

#### Scenario: Entries are RDF
- **WHEN** an entry is read back
- **THEN** it is an RDF dataset with one named graph per entry, using terms of the `vt:` runtime ontology

### Requirement: Append Fencing Token
A journal store append SHALL accept optional append metadata carrying a fencing token made of a scope IRI and a monotonically increasing epoch; a store that declares fencing support SHALL reject an append whose epoch is lower than the highest epoch it has accepted for that scope, and an append without a token SHALL behave exactly as an unfenced append.

#### Scenario: Unfenced append unchanged
- **WHEN** a single-node runtime appends without a fencing token
- **THEN** the append is checked only by compare-and-set on the expected position

#### Scenario: Stale epoch rejected
- **WHEN** a store has accepted an append with token (shard S, epoch 5) and then receives an append with token (shard S, epoch 4)
- **THEN** the append fails with a fencing error and writes nothing, even if its expected position matches

#### Scenario: Fencing support is declared
- **WHEN** the runtime opens a journal store
- **THEN** it can read whether the store enforces fencing tokens before relying on them

### Requirement: Graph Store Contract
A graph store SHALL accept RDF quads and answer SPARQL 1.1 queries, and SHALL declare whether it supports RDF 1.2 triple terms; it SHALL NOT be required to provide compare-and-set or transactions across requests.

#### Scenario: Query current state
- **WHEN** a SPARQL query asks for the active states of an actor in its current-state graph
- **THEN** the graph store returns them

#### Scenario: Capability is declared
- **WHEN** the runtime opens a graph store
- **THEN** it can read whether that store supports RDF 1.2 triple terms before writing any annotations

### Requirement: Shared Backend Test Kit
The storage contracts SHALL come with a test kit that every backend MUST pass, covering append atomicity, compare-and-set conflicts under concurrency, read-after-append, quad round trips and the declared RDF capability.

#### Scenario: Backend admitted by the kit
- **WHEN** a new backend implementation runs the test kit
- **THEN** it is accepted only if every test passes, including concurrent conflicting appends

### Requirement: Tiramemsu Backend
The default backend SHALL implement both the journal store and the graph store on tiramemsu, holding one shard's actors in one tiramemsu file, so that the journal and the current-state graph live in one consistent store.

#### Scenario: Both traits from one file
- **WHEN** an application opens the tiramemsu backend on a shard file
- **THEN** it obtains a journal store and a graph store that read and write the same file

#### Scenario: Compare-and-set inside one transaction
- **WHEN** an append runs on the tiramemsu backend
- **THEN** reading the actor's journal head, checking the expected position and writing the entries happen in one tiramemsu transaction on its single writer

### Requirement: Input And State In One Transaction
On the tiramemsu backend, a journal entry and the changes it causes to the actor's current-state graph SHALL commit in the same transaction, so that the current-state graph is never ahead of or behind the journal.

#### Scenario: No projection lag
- **WHEN** an append commits on the tiramemsu backend
- **THEN** a query on the current view immediately sees the new configuration of the actor

#### Scenario: Rolled back together
- **WHEN** the transaction fails after writing the entry but before the state changes
- **THEN** neither the entry nor the state changes are visible

### Requirement: State Facts Carry Their Cause
On the tiramemsu backend, every current-state statement written by a macrostep SHALL be a derived statement whose support includes the journal entry of that macrostep, so that the input that caused a state can be found from the state.

#### Scenario: Why is the actor in this state
- **WHEN** an operator asks which input made an actor's state `Shipped` active
- **THEN** following the support of the active-state statement leads to the journal entry and its input event

#### Scenario: Superseded state keeps history
- **WHEN** a later macrostep exits `Shipped`
- **THEN** the active-state statement is retracted, not deleted, and remains visible in history views

### Requirement: Time Travel Views
On the tiramemsu backend, the current-state graph SHALL be readable at the latest transaction and as of any earlier transaction time, using the store's own views rather than replay.

#### Scenario: State as of a past transaction
- **WHEN** a SPARQL query on an as-of view at transaction t asks for an actor's active states
- **THEN** it returns the states that were active after the last append committed at or before t

### Requirement: One File Per Shard
The tiramemsu backend SHALL map each shard to its own file, and SHALL NOT serialize appends of actors in different shards through one writer.

#### Scenario: Shards append independently
- **WHEN** actors in two different shards append at the same time
- **THEN** neither append waits for the other's writer

#### Scenario: Single node uses a configurable shard count
- **WHEN** a single-node runtime starts with a shard count of 8
- **THEN** actors are placed in 8 files by a hash of their address IRI

### Requirement: SPARQL Graph Store Adapter
A second backend SHALL implement the graph store contract on Oxigraph and on any SPARQL 1.1 Query and Update endpoint, and SHALL NOT implement the journal store contract.

#### Scenario: Journal elsewhere, graph in Oxigraph
- **WHEN** an application configures tiramemsu as journal store and Oxigraph as graph store
- **THEN** current-state graphs and dead letters are queryable in Oxigraph, and journal correctness depends only on tiramemsu

#### Scenario: Remote endpoint
- **WHEN** the graph store is a remote SPARQL endpoint URL
- **THEN** the adapter writes quads by SPARQL Update and answers queries by SPARQL Query

### Requirement: Eventual Projection To External Graph Stores
When the graph store is separate from the journal store, the runtime SHALL project journal entries into it in journal order, idempotently, and SHALL record per actor the last projected position.

#### Scenario: Projection catches up
- **WHEN** the external graph store was unreachable while 50 entries were appended
- **THEN** once it is reachable the projector writes those 50 entries in order and records the new position

#### Scenario: Re-projection is idempotent
- **WHEN** the projector writes an entry it had already projected
- **THEN** the graph store contents are unchanged

### Requirement: RDF 1.1 Reification Fallback
When a graph store does not support RDF 1.2 triple terms, the runtime SHALL write each annotated statement in classic `rdf:Statement` reification, and SHALL read it back as the same annotation.

#### Scenario: Annotation on an RDF 1.1 store
- **WHEN** the runtime annotates a transition statement with a confidence on a store that lacks RDF 1.2 support
- **THEN** it writes an `rdf:Statement` with subject, predicate and object plus the confidence, and a later read returns the same annotation
