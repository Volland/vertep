## ADDED Requirements

### Requirement: One Commit Per Turn
A journal store SHALL accept, with an append, the turn's transition annotations, the outbox mark, the acknowledgement of the triggering message when its mailbox is in the same store, and a snapshot when one falls due. It SHALL write them in the append's transaction, so a turn commits once and is all or nothing. A store that cannot hold one of them MUST declare it in its capabilities, and the runtime then writes it separately after the append.

#### Scenario: Turn is one transaction
- **WHEN** a turn's append carries annotations, an outbox mark and a message acknowledgement on the tiramemsu backend
- **THEN** all of them are written in the transaction that checks the head, and the store records one commit

#### Scenario: Conflict writes none of it
- **WHEN** the head check of such an append fails
- **THEN** neither the entry, nor the annotations, nor the outbox mark, nor the acknowledgement is written

#### Scenario: Store without the capability
- **WHEN** a journal store declares that it cannot carry annotations in an append
- **THEN** the runtime writes them after the append, as it writes state for stores without state in the append

### Requirement: Group Commit Per Shard
A journal store SHALL accept a batch of appends for several actors of one shard in one call, check every head and fencing token inside one transaction, and write the appends that pass. It MUST return one result per actor, and one actor's conflict MUST NOT prevent the others from being written. The runtime SHALL acknowledge and dispatch each actor's turn only after the batch is committed.

#### Scenario: Batch of independent actors
- **WHEN** turns of three actors in one shard are appended as one batch
- **THEN** the store commits once and all three entries are written

#### Scenario: One conflict in a batch
- **WHEN** one actor in a batch of three has a stale expected position
- **THEN** that actor's result is a conflict, nothing of it is written, and the other two are written in the same commit

#### Scenario: No dispatch before the batch commits
- **WHEN** a batch is being committed
- **THEN** none of its turns' sends are dispatched and none of its messages are acknowledged until the commit returns

### Requirement: Unversioned Bookkeeping
The tiramemsu backend SHALL keep mailbox rows, outbox marks, timer dispatch marks, actor heads, accepted message ids and snapshots as unversioned rows that are freed when they are cleared. Journal entries, current state, annotations and dead letters SHALL stay versioned RDF.

#### Scenario: Acknowledged message leaves no row
- **WHEN** a message in a tiramemsu-backed mailbox is acknowledged
- **THEN** no row for it remains in the store, including history

#### Scenario: Outbox mark does not accumulate
- **WHEN** an actor's outbox mark advances a thousand times
- **THEN** the store holds one mark for that actor

#### Scenario: Existing files migrate once
- **WHEN** this version opens a file written by an earlier version with outbox marks and mailbox rows as triples
- **THEN** they are copied to unversioned rows once, the migration is recorded, and a second open copies nothing

### Requirement: Durable Commit
A journal store SHALL declare whether a returned append is durable against an operating-system crash. The tiramemsu backend SHALL open journal shards so that it is durable by default. A relaxed mode MUST be chosen explicitly and MUST be declared as not durable.

#### Scenario: Default is durable
- **WHEN** a tiramemsu backend is opened without a durability option
- **THEN** its capabilities declare durable commits and each commit is synced before the append returns

#### Scenario: Relaxed mode is declared
- **WHEN** an application opens the backend in relaxed mode
- **THEN** its capabilities declare commits as not durable and the runtime documents the risk in its startup log

### Requirement: Journal Range Read
A journal store SHALL read up to a given number of an actor's entries from a given position (`JournalStore::read_range`, added for journal shipping in M13), and the tiramemsu backend MUST answer such a read from an index on position, without visiting entries outside the range.

#### Scenario: Read after a snapshot
- **WHEN** ten entries from position 991 of an actor with 100,000 entries are read on the tiramemsu backend
- **THEN** exactly those ten entries are returned and the read's cost does not depend on the other entries

#### Scenario: Default for other backends
- **WHEN** a journal store implements only the whole-journal read
- **THEN** the range read still returns the right entries through the default implementation

### Requirement: Message Id Lookup
A journal store SHALL answer whether a message id was accepted for an actor. The tiramemsu backend MUST answer from an index written in the append transaction.

#### Scenario: Accepted id is found
- **WHEN** an append records message id m for actor A and the lookup asks for (A, m)
- **THEN** the answer is yes, without reading A's entries

#### Scenario: Same id for another actor
- **WHEN** the lookup asks for (B, m) and m was accepted only by A
- **THEN** the answer is no
