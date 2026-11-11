## Purpose

Makes actor state event-sourced: a virtual actor's state is the deterministic replay of a journal of the inputs it accepted, appended atomically with compare-and-set, deduplicated by message id, with outgoing sends dispatched through a transactional outbox.

## ADDED Requirements

### Requirement: Journal Of Accepted Inputs
The runtime SHALL record, for every virtual actor, an append-only journal whose entries each contain the external input event that started a macrostep (name, payload, message id, origin, origin type), the chart version it ran under, and the effects that macrostep produced.

#### Scenario: One entry per turn
- **WHEN** a virtual actor completes a macrostep for an external event
- **THEN** exactly one journal entry is appended containing that event, its message id and the chart version IRI

#### Scenario: Entries are never rewritten
- **WHEN** any journal entry has been appended
- **THEN** no later operation changes or deletes it; corrections are new entries

### Requirement: Journal Entry Content
Each journal entry SHALL record the outgoing sends (with target, event, payload and send id), the timers scheduled or cancelled, the resulting configuration digest, and the journal position (a per-actor sequence number starting at 1 with no gaps).

#### Scenario: Entry carries its effects
- **WHEN** a macrostep sends two events and schedules one delayed send
- **THEN** its entry lists both sends and the timer, each with a stable id

#### Scenario: Gap-free positions
- **WHEN** three entries are appended for an actor
- **THEN** their positions are 1, 2 and 3

### Requirement: Compare-And-Set Append
Appending a journal entry SHALL succeed only if the actor's journal is still at the expected position; otherwise the append SHALL fail with a conflict and write nothing.

#### Scenario: Append at the expected position
- **WHEN** an activation appends with expected position 7 to a journal whose last entry is 7
- **THEN** the entry is written at position 8

#### Scenario: Conflicting append rejected
- **WHEN** two activations of the same actor both try to append with expected position 7
- **THEN** exactly one append succeeds and the other fails with a conflict and leaves no trace

#### Scenario: Losing activation steps down
- **WHEN** an activation's append fails with a conflict or a fencing error
- **THEN** that activation discards its in-memory state, does not dispatch any of the macrostep's effects, and is deactivated

#### Scenario: Fencing token passed through
- **WHEN** the runtime is configured with a fencing token for the actor's shard
- **THEN** every append for that actor carries the token, and the store's fencing check applies in addition to compare-and-set

### Requirement: Message Deduplication
The runtime SHALL accept each message id at most once per actor; a message whose id already appears in the actor's journal SHALL be acknowledged and dropped without running a macrostep.

#### Scenario: Redelivered message dropped
- **WHEN** a mailbox redelivers a message whose id is already journaled for the receiver
- **THEN** the message is acknowledged, no macrostep runs and no entry is appended

#### Scenario: Same id to different actors
- **WHEN** the same message id is delivered to two different actors
- **THEN** each actor processes it once

### Requirement: Transactional Outbox
Outgoing sends of a macrostep SHALL be dispatched only after the entry that records them has been appended, and the dispatcher SHALL retry each recorded send until its transport accepts it, so that every recorded send is delivered at least once and no unrecorded send is ever delivered.

#### Scenario: Crash after append, before dispatch
- **WHEN** the node crashes after a journal entry with two sends was appended but before they were dispatched
- **THEN** after restart both sends are dispatched with their original send ids

#### Scenario: Crash before append
- **WHEN** the node crashes during a macrostep, before its entry was appended
- **THEN** none of that macrostep's sends are dispatched, and the triggering message is redelivered

#### Scenario: Stable send ids enable receiver dedup
- **WHEN** a send is dispatched twice because of a retry
- **THEN** both copies carry the same message id and the receiver processes it once

### Requirement: Deterministic Replay
Replaying an actor's journal from its initial configuration, or from a snapshot, with the chart version recorded in each entry SHALL produce exactly the configuration, datamodel and effects that were recorded.

#### Scenario: Replay equals recorded state
- **WHEN** an actor is reactivated by replaying all of its journal entries
- **THEN** its configuration and datamodel equal those at the time of the last entry, and each replayed macrostep produces the recorded sends and timers

#### Scenario: Replay divergence detected
- **WHEN** a replayed macrostep produces a configuration digest different from the recorded one
- **THEN** activation fails with a replay-divergence error naming the actor and position, and the actor is not activated

#### Scenario: Replay produces no external effects
- **WHEN** entries are replayed during activation
- **THEN** no send is dispatched again by replay; only undispatched outbox entries are dispatched

### Requirement: Inputs Only From The Journal
A macrostep SHALL observe no input other than the journaled event and the actor's state: wall-clock time, randomness and I/O SHALL enter only as values recorded in journal entries.

#### Scenario: Time is a journaled value
- **WHEN** executable content reads the current time
- **THEN** it receives the timestamp recorded with the triggering input, both originally and on replay

#### Scenario: Random values are recorded
- **WHEN** a chart requests a random value
- **THEN** the value is recorded in the journal entry and replay returns the same value

### Requirement: External Results As Inputs
Results of external work, including tool calls and LLM completions, SHALL reach an actor only as input events that are journaled with their full content, so that replay never repeats the external call.

#### Scenario: LLM result is journaled
- **WHEN** an invoked LLM actor returns a completion to its parent
- **THEN** the parent's journal entry contains the completion text and metadata as the input event

#### Scenario: Replay does not call the model
- **WHEN** the parent is replayed
- **THEN** the completion is read from the journal and no model is called

### Requirement: Snapshots
The runtime SHALL write snapshots of an actor's configuration, datamodel, pending deferred events and active timers at a journal position, and SHALL activate from the latest valid snapshot plus the entries after it.

#### Scenario: Activation from snapshot
- **WHEN** an actor has a snapshot at position 1000 and entries up to 1010
- **THEN** activation loads the snapshot and replays only entries 1001 to 1010

#### Scenario: Snapshot is checked against replay
- **WHEN** a snapshot's chart version differs from the version of the entry at its position, or its digest does not match
- **THEN** the snapshot is ignored and activation falls back to an earlier snapshot or to full replay

#### Scenario: Snapshot policy is configurable
- **WHEN** an application sets a snapshot interval of N entries
- **THEN** a snapshot is written at least every N entries per actor

### Requirement: Journal Read Access
The runtime SHALL let callers read an actor's journal from a position, and SHALL let them reconstruct the actor's state as of any journal position without changing the journal.

#### Scenario: Time travel to a position
- **WHEN** a caller asks for the state of an actor as of position 42
- **THEN** the runtime returns the configuration and datamodel after entry 42, computed by replay, and the live actor is unaffected

#### Scenario: Fork from a position
- **WHEN** a caller forks an actor at position 42 under a new address
- **THEN** a new virtual actor exists whose journal starts with a reference to the source actor and position, and both evolve independently
