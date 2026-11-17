## ADDED Requirements

### Requirement: Parallel Standby Shipping
With a replication factor above one, the shard owner SHALL ship an append to all of the shard's standbys concurrently and acknowledge it once the configured number have stored it. It SHALL skip a pre-flight exchange with a standby whose acknowledged head equals the owner's head before the append.

#### Scenario: Latency is the slowest needed standby, not the sum
- **WHEN** an append is shipped to two standbys with one-way delays of 5 ms and 8 ms and both must acknowledge
- **THEN** the append is acknowledged after about 8 ms of shipping, not 13 ms

#### Scenario: Caught-up standby needs no pre-flight
- **WHEN** a standby has acknowledged every entry up to the owner's head
- **THEN** the next append is shipped to it without a pre-flight exchange

#### Scenario: Lagging standby still catches up
- **WHEN** a standby's acknowledged head is behind the owner's
- **THEN** the owner ships the missing entries before the new one, and the standby holds every acknowledged entry in order
