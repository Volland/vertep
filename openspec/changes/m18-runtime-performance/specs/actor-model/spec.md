## ADDED Requirements

### Requirement: Shard Groups
A process SHALL be able to run several single-threaded nodes as a group over one storage. Each node in the group owns a fixed set of shards, determined by the shard number, and runs only the virtual actors of those shards. An activation MUST stay on its node's thread. A send to an actor of another group's shard SHALL reach that group without blocking the sender's thread.

#### Scenario: Two shards, two threads
- **WHEN** a group of two nodes runs actors A in shard 0 and B in shard 1, and both have messages
- **THEN** A's and B's turns run concurrently on different threads, and each actor's turns stay in its mailbox order

#### Scenario: Cross-group send
- **WHEN** actor A in group 0 sends an event to actor B in group 1
- **THEN** B receives it as an external event with A as its origin, exactly as within one node

#### Scenario: One group is today's node
- **WHEN** an application builds a group of one node
- **THEN** behaviour, determinism and the virtual-clock tests are the same as for a single node

### Requirement: Deactivation Releases Memory
When a virtual actor is deactivated, the node SHALL release every per-actor structure it holds: the activation, its mailbox handle when the mailbox is empty, its outbox record when nothing is owed, and its deduplication window. Per-address statistics MUST NOT grow with the number of distinct actors ever activated.

#### Scenario: Churned actors leave nothing
- **WHEN** 100,000 distinct actors are each activated once and deactivated
- **THEN** the node holds no per-actor entry for them, and a later message to any of them reactivates it with its state from the journal

### Requirement: Idle Detection Without Scanning
The node SHALL find actors whose idle timeout has passed, and its next deadline, without visiting every active actor in every scheduling round.

#### Scenario: Many active, few idle
- **WHEN** 100,000 actors are active and one of them passes its idle timeout
- **THEN** the round that deactivates it visits that actor and not the other 99,999
