## MODIFIED Requirements

### Requirement: Ordering Per Sender And Receiver
The cluster transport SHALL deliver messages from one sending actor to one receiving actor in the order they were sent, including when messages are retried, and it MUST NOT impose any order between messages of different sender and receiver pairs.

#### Scenario: Two sends arrive in order
- **WHEN** actor X sends events e1 and then e2 to actor Y on another node
- **THEN** Y's mailbox receives e1 before e2

#### Scenario: Independent pairs do not block each other
- **WHEN** delivery from X to Y is stalled by a full mailbox
- **THEN** messages from X to Z and from W to Y keep flowing

#### Scenario: Retry keeps the order
- **WHEN** e1 from X to Y fails once and is retried while e2 from X to Y is queued behind it
- **THEN** Y's mailbox receives e1 before e2

## ADDED Requirements

### Requirement: Stream Budget Within The Peer Limit
The cluster transport SHALL never need more concurrent streams to one peer than that peer allows. It SHALL set its own stream limit explicitly, keep its pair streams below it, and share streams between pairs past that cap while keeping each pair ordered. Connecting to a peer or opening a stream MUST NOT block sends to other peers.

#### Scenario: More pairs than the limit
- **WHEN** 1,000 distinct sender and receiver pairs send to actors on one peer whose stream limit is 256
- **THEN** every message is delivered, each pair in order, and no send waits for a stream that cannot open

#### Scenario: Slow peer does not stall others
- **WHEN** the handshake with peer P hangs
- **THEN** messages to other peers keep flowing

### Requirement: Pipelined Pair Streams
A pair stream SHALL keep up to a configured number of frames in flight before it waits for their replies. It SHALL read replies in send order, and SHALL retry from the first unacknowledged frame with exponential back-off until each frame's delivery deadline.

#### Scenario: Window fills the round trip
- **WHEN** X sends 100 events to Y with a window of 16
- **THEN** at most 16 frames are unacknowledged at any time, all 100 arrive in order, and the elapsed time is well below 100 round trips

#### Scenario: Deadline still applies
- **WHEN** a frame in the window cannot be acknowledged before its delivery deadline
- **THEN** retries for it stop and the sending actor receives a delivery error event, as for an unpipelined send
