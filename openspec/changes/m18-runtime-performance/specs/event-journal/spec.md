## MODIFIED Requirements

### Requirement: Transactional Outbox
Outgoing sends of a macrostep SHALL be dispatched only after the entry that records them has been appended and the append is durable under the store's declared durability. The dispatcher SHALL retry each recorded send until its transport accepts it, so that every recorded send is delivered at least once and no unrecorded send is ever delivered.

#### Scenario: Crash after append, before dispatch
- **WHEN** the node crashes after a journal entry with two sends was appended but before they were dispatched
- **THEN** after restart both sends are dispatched with their original send ids

#### Scenario: Crash before append
- **WHEN** the node crashes during a macrostep, before its entry was appended
- **THEN** none of that macrostep's sends are dispatched, and the triggering message is redelivered

#### Scenario: Stable send ids enable receiver dedup
- **WHEN** a send is dispatched twice because of a retry
- **THEN** both copies carry the same message id and the receiver processes it once

#### Scenario: No send outlives its entry
- **WHEN** an append on a store that declares durable commits returns, its sends are dispatched, and the operating system then crashes
- **THEN** after restart the entry is in the journal, so no receiver holds a message its sender's history lacks

## ADDED Requirements

### Requirement: Activation Reads From The Snapshot
Activation SHALL read the actor's latest valid snapshot and only the journal entries after it, and the outbox replay SHALL read only the entries after the outbox mark. Neither MUST read the journal from its first entry when a valid snapshot or mark exists.

#### Scenario: Long history, recent snapshot
- **WHEN** an actor with 100,000 entries and a valid snapshot at 99,990 is activated
- **THEN** activation reads the snapshot and entries 99,991 to 100,000, and the actor's state equals full replay's

#### Scenario: Invalid snapshot falls back
- **WHEN** the latest snapshot does not match the entry at its position
- **THEN** activation tries the next older snapshot kept, and full replay only when none is valid

### Requirement: Snapshot Retention
The runtime SHALL keep the newest `snapshot_keep` snapshots of each actor, at least one, and remove older ones when a new snapshot is written. Snapshots are derived from the journal, so removing one MUST NOT lose information.

#### Scenario: Old snapshots are pruned
- **WHEN** an actor with `snapshot_keep = 2` and a snapshot interval of 100 reaches position 1,000
- **THEN** the store holds its snapshots at 900 and 1,000 only

#### Scenario: Pruned history replays
- **WHEN** a caller asks for the actor's state at position 150 after the snapshot at 100 was pruned
- **THEN** the state is computed by replay and equals the state recorded at that position

### Requirement: Indexed Deduplication
Message deduplication SHALL stay exact (each message id accepted at most once per actor), and the runtime SHALL NOT keep every accepted id in memory. It SHALL check a bounded window of recently accepted ids and otherwise ask the journal store's message-id lookup.

#### Scenario: Old duplicate after reactivation
- **WHEN** an actor that accepted message m 10,000 entries ago is reactivated and m is redelivered
- **THEN** m is acknowledged and dropped without a macrostep, and activation did not collect every past id

#### Scenario: Memory is bounded by the window
- **WHEN** an active actor accepts many more messages than the dedup window
- **THEN** its in-memory id set never holds more than the window
