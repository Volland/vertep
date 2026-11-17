## ADDED Requirements

### Requirement: WebSocket Live Delivery
A relay SHALL deliver mailbox messages to a connected party over WebSocket using the same envelope and acknowledgement as polling, MUST fall back to polling when the socket is unavailable, and MUST NOT deliver a message twice after an acknowledgement.

#### Scenario: Live delivery
- **WHEN** a party is connected over WebSocket and a message arrives
- **THEN** the message is delivered without a poll and is removed after acknowledgement

#### Scenario: Fallback to polling
- **WHEN** the socket drops
- **THEN** the party receives the pending messages by polling

#### Scenario: No double delivery
- **WHEN** a message was acknowledged over the socket and the party then polls
- **THEN** the message is not delivered again
