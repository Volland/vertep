# journal-shipping Specification

## Purpose
Replicates a journal between cluster nodes with a wire protocol that preserves entry order and digests, resumes after interruption and bounds memory under a slow follower, closing the gap noted in M4.

## Requirements

### Requirement: Ordered Shipping
A follower SHALL pull a journal's entries after its last accepted position over the cluster transport, receive them in position order with their digests, and MUST reject an entry whose position or digest does not extend its journal.

#### Scenario: A follower catches up
- **WHEN** a follower with positions 1 to 10 pulls from a leader at position 25
- **THEN** it receives entries 11 to 25 in order and its journal verifies

#### Scenario: A gap is refused
- **WHEN** an entry arrives whose position is not the follower's next
- **THEN** the follower rejects it and re-pulls from its last accepted position

#### Scenario: A bad digest is refused
- **WHEN** an entry's digest does not match its content
- **THEN** the follower rejects it and does not append

### Requirement: Resumption
Shipping SHALL resume from the follower's last durable position after a dropped connection or a follower restart, without re-applying an entry.

#### Scenario: Connection drops mid-ship
- **WHEN** the connection fails after entry 17 of 25
- **THEN** the follower resumes from 18 and ends with 25 entries, each applied once

#### Scenario: Follower restarts
- **WHEN** a follower restarts after durably appending entry 17
- **THEN** it resumes from 18

### Requirement: Back Pressure
A leader SHALL send no more than the follower has acknowledged room for, so that a slow follower bounds the leader's memory use.

#### Scenario: A slow follower
- **WHEN** a follower acknowledges slowly
- **THEN** the leader's buffered bytes for it stay within the configured window
