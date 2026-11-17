## ADDED Requirements

### Requirement: Receive Cost Under Deferral
A mailbox SHALL NOT examine a deferred message again until the receiver's deferring descriptors change. The cost of receiving a deliverable message MUST NOT grow with the number of deferred messages queued before it. Deferred messages SHALL keep the order [[openspec:messaging#Deferred Events]] requires.

#### Scenario: Many deferred, one deliverable
- **WHEN** 10,000 deferred messages are queued and a non-deferred message arrives
- **THEN** receive returns the new message without matching the 10,000 against the descriptors again

#### Scenario: Leaving the deferring state
- **WHEN** the actor's active states stop deferring `order.*`
- **THEN** the deferred `order.*` messages are offered again in arrival order, before newer messages that arrived after them

### Requirement: Bounded Mailboxes
A mailbox route SHALL accept an optional capacity and an overflow policy `reject` or `wait`. When a mailbox is full:
- under `reject`, an enqueue fails with a full error; the outbox treats it as a refused delivery and retries after its back-off, and an outside send returns the full error to its caller;
- under `wait`, the cluster transport's backpressure applies.

No accepted message MUST be dropped because a mailbox is full.

#### Scenario: Full mailbox rejects an outside send
- **WHEN** an outside caller sends to an actor whose mailbox is at capacity under `reject`
- **THEN** the send returns a full error and the mailbox is unchanged

#### Scenario: Outbox retries a full mailbox
- **WHEN** an actor's recorded send meets a full mailbox under `reject`
- **THEN** the send stays owed and is delivered after the receiver drains below capacity

#### Scenario: Capacity from configuration
- **WHEN** a mailbox configuration routes a prefix with `capacity: 1000` and `overflow: reject`
- **THEN** the built router enforces that capacity on those addresses only

### Requirement: Durability By Origin
A mailbox route MAY choose durability by origin.
- A message whose id is a recorded send of an actor's journal SHALL then wait in memory, and the sender SHALL keep the send owed until the receiver confirms it has journaled the turn that consumed it.
- A message with no recorded sender SHALL be stored durably.

After a crash, the messages held in memory MUST be delivered again by their senders' outbox replay, and deduplication MUST drop any copy already consumed.

#### Scenario: Receiver crash loses nothing
- **WHEN** a receiver node crashes while messages from actors wait in its in-memory part of a by-origin mailbox
- **THEN** after restart the senders re-send them, each is consumed once, and per-sender order is kept

#### Scenario: Outside message is stored
- **WHEN** an outside caller sends to an address whose route uses durability by origin
- **THEN** the message is stored durably and survives a restart of the receiver

#### Scenario: Owed until consumed
- **WHEN** a receiver defers a message from actor S
- **THEN** S keeps that send owed and its outbox mark does not advance past it until the receiver consumes it
