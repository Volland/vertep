## Purpose

Defines protocol instances: a signed genesis, countersigned joins, role-derived authority, pairwise sealed fan-out through any relay, hash-linked history, sub-protocol invocation, and the binding of charts and mailboxes to roles.

## ADDED Requirements

### Requirement: Protocol Instance
An instance SHALL begin with a genesis message signed by the initiator that names the protocol identity, an instance id and a binding from each role to a DID. The instance id SHALL be the genesis hash. A genesis for a child instance SHALL also name the parent message that opened it.

#### Scenario: Genesis creates an instance
- **WHEN** an initiator sends a genesis binding two roles
- **THEN** the instance id equals the genesis hash, and each participant can verify the binding

#### Scenario: Unsigned genesis refused
- **WHEN** a genesis has no valid signature from a DID bound in it
- **THEN** it is refused

### Requirement: Joining
A participant other than the initiator SHALL join with a signed `join`, or decline with a signed `decline`, before any other message of the instance from it is accepted. The instance SHALL be open once every required role has joined. A message from a role that has not joined SHALL be refused.

#### Scenario: Instance opens
- **WHEN** both bound roles have joined
- **THEN** the instance is open and messages are checked against the projection

#### Scenario: Decline ends the instance
- **WHEN** a required role declines
- **THEN** the instance never opens and the initiator is told

#### Scenario: No join, no standing
- **WHEN** a bound DID sends a message before joining
- **THEN** it is refused

### Requirement: Join Policy
An agent SHALL apply its own policy to a genesis before it journals it: which protocols, which roles, which initiators, and optionally which credentials it accepts. The default policy SHALL refuse every genesis. A refused genesis SHALL cost the receiver no journal append.

#### Scenario: Default refuses
- **WHEN** an agent with the default policy receives a genesis
- **THEN** it is refused and nothing is appended

#### Scenario: Policy admits a known initiator
- **WHEN** the policy allows protocol P in role R from initiator I and I sends a genesis
- **THEN** the genesis is accepted and journaled

### Requirement: Role-Derived Authority
A message SHALL be authorised when its sender is bound to a role in an open instance and the role's projection allows the message in the current state. No other grant SHALL be required, and no grant SHALL widen a role's messages beyond the projection.

#### Scenario: Role grants exactly the projection
- **WHEN** a bound sender sends a message its role may send in this state
- **THEN** it is authorised without any grant

#### Scenario: Grant does not widen
- **WHEN** a configured DID grant covers an event the role may not send in this state
- **THEN** the protocol message is still refused

### Requirement: Pairwise Sealed Fan-Out
A message SHALL reach each recipient in its recipient set as a separate copy, sealed to that recipient, through any mailbox or relay. A relay or mailbox SHALL hold only items sealed to its owner. A message with a recipient set smaller than the roles SHALL be unreadable by the other roles.

#### Scenario: Only the recipient can read
- **WHEN** a message is sealed to two of three roles
- **THEN** the third role's mailbox never holds a copy it can open

#### Scenario: Any transport carries the envelope
- **WHEN** the same signed message is carried over TSP, DIDComm and a local channel
- **THEN** each receiver reaches the same verdict

### Requirement: Hash-Linked History
Each message SHALL list the hashes of the messages it depends on, so an instance's history is a hash-linked graph. A participant SHALL be able to compare heads with another to find a missing or divergent message. A message hidden from a participant SHALL appear in its history as its hash only. A participant SHALL forward to a late joiner only what the joiner may read, and the joiner SHALL verify it against the hashes.

#### Scenario: Divergence found
- **WHEN** two participants compare heads and one lacks a message
- **THEN** the missing hash is named

#### Scenario: Hidden message keeps the chain
- **WHEN** a late joiner receives a history with one message hidden from it
- **THEN** the hash chain still verifies end to end

### Requirement: Sub-Protocol Invocation
A transition of a protocol MAY open a child instance of another protocol. The child's role bindings SHALL come from the parent's roles, and a participant already in the parent SHALL need no new join unless the child's policy requires one. The parent SHALL wait in its state for the child's completion message, signed under the child protocol. A space SHALL be the tree of instances under one root genesis, with one shared hash-linked history, and each checker SHALL see one protocol only.

#### Scenario: Child completes the parent step
- **WHEN** a child instance reaches a final state and sends its completion message
- **THEN** the parent leaves its waiting state

#### Scenario: Bindings come from the parent
- **WHEN** a child names a role bound to a DID that is not a parent participant
- **THEN** the child's genesis is refused

### Requirement: Chart Role Binding
A chart SHALL declare the protocol and role it plays. The agent's `VertepProtocol` service entry SHALL be generated from these declarations. The mailbox SHALL run the conformance check before any journal append, at the point where grants are enforced, and SHALL journal the genesis, the joins and each accepted message with its verdict; replay SHALL not verify again. The chart SHALL see an ordinary event named by the message type, with the verified sender as origin, and the instance id and role available to guards. The send path SHALL run the same check on the actor's own projected state, and a send the role may not make SHALL fail locally. The checker state SHALL be derived on replay and SHALL not be stored.

#### Scenario: Inbound check before the journal
- **WHEN** a message is refused by the conformance check
- **THEN** nothing is appended to the actor's journal and the refusal is recorded

#### Scenario: Outbound send refused locally
- **WHEN** a chart sends a message its role may not send in the current state
- **THEN** the send fails with an error and no message leaves the actor

#### Scenario: Replay without re-verification
- **WHEN** an actor is replayed from its journal
- **THEN** it reaches the same instance state without resolving any DID or verifying any signature
