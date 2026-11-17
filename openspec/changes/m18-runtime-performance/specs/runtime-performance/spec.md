## Purpose

Measures the runtime's cost and states its limits: benchmarks of the hot paths, operation budgets checked by counting store calls, a regression gate, and bounded memory and storage over long runs.

## ADDED Requirements

### Requirement: Hot Path Benchmarks
The project SHALL provide benchmarks for the runtime's hot paths that run with one command:
- turns per second on a memory and on a tiramemsu store, with and without a send;
- activation time against journal length, with and without a snapshot;
- mailbox receive with deferred messages queued;
- cross-node send round trip.

Each result MUST record the commit, the machine and the store's durability mode.

#### Scenario: One command runs the suite
- **WHEN** a contributor runs the documented benchmark command on a clean checkout
- **THEN** every listed case runs and its result is written with the commit, the machine and the durability mode

#### Scenario: Activation is measured against history
- **WHEN** the activation benchmark runs at 10, 1,000 and 100,000 entries with a snapshot ten entries before the head
- **THEN** it reports one result per length, so a cost that grows with history is visible

### Requirement: Operation Budgets
The runtime's cost per operation SHALL be stated as counts of store calls and checked by tests through a counting storage wrapper, so the checks hold on any machine. The budgets:
- a turn of a virtual actor whose mailbox is in its own shard commits once;
- activation reads the latest snapshot and only the entries after it;
- a duplicate message is dropped without reading the journal from the start.

#### Scenario: A turn with a send commits once
- **WHEN** a virtual actor with a store mailbox in its own shard takes one turn that sends one event, on a tiramemsu store wrapped by the counting storage
- **THEN** the wrapper records exactly one commit for that turn

#### Scenario: Activation reads only after the snapshot
- **WHEN** an actor with 1,000 entries and a snapshot at position 990 is activated through the counting storage
- **THEN** the wrapper records one snapshot read and a journal read of entries 991 to 1,000 only

#### Scenario: Budget breach fails the build
- **WHEN** a change makes a turn commit twice
- **THEN** the budget test fails and names the operation and both counts

### Requirement: Benchmark Regression Gate
Continuous integration SHALL compare benchmark results with the last recorded baseline and fail when a hot-path benchmark regresses beyond its stated threshold. A change MAY update the baseline only together with a recorded reason.

#### Scenario: Regression blocks the merge
- **WHEN** turns per second on the tiramemsu store drop by more than the stated threshold against the baseline
- **THEN** the gate fails and reports the case, the baseline and the new result

#### Scenario: Accepted baseline change
- **WHEN** a change moves a baseline on purpose, such as turning on immediate durability
- **THEN** the new baseline is recorded with its reason and later runs compare against it

### Requirement: Bounded Long-Run Growth
Over a long run, the runtime's memory SHALL stay bounded by the number of actors active now. Its storage SHALL grow only by journal entries, current state, annotations, dead letters and the snapshots kept. Acknowledged mailbox rows, replaced outbox marks and pruned snapshots MUST leave no rows behind.

#### Scenario: Soak with activation churn
- **WHEN** a soak test sends one million messages to one hundred thousand virtual actors with an idle timeout that deactivates most of them between messages
- **THEN** at the end the node holds per-actor entries only for active actors, and the store holds no mailbox or outbox rows for acknowledged messages

#### Scenario: Storage per message is the record
- **WHEN** the soak test's store size is compared before and after a run of N acknowledged messages that cause no dead letters
- **THEN** the growth is accounted for by the N journal entries with their state changes and annotations, and the snapshots kept
