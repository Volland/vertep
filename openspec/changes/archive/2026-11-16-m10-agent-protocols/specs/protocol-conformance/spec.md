## Purpose

Defines the signed message envelope, the pure conformance function that judges each message against a role's projection, causal delivery, violation records, and the test vectors and Lean model that make the protocol verifiable by any implementation.

## ADDED Requirements

### Requirement: Signed Message Envelope
Every protocol message SHALL carry a body signed by the sender's DID key, as a detached signature made before any transport seals it. The signed body SHALL contain the protocol identity, the instance id, the sender's role, a per-role sequence number, the hashes of the messages it depends on, the recipient set, the message type and the payload. The message hash SHALL cover the body and the signature. A transport SHALL seal the signed message to each recipient and SHALL not change it.

#### Scenario: Signature verifies for a third party
- **WHEN** a recipient forwards a decrypted message to another participant
- **THEN** that participant verifies the signature against the sender's DID document without the transport

#### Scenario: Tampered body refused
- **WHEN** one byte of the body changes after signing
- **THEN** verification fails

### Requirement: Conformance Check
The crate SHALL provide a pure function `accept(state, message)` that returns a verdict without any I/O. It SHALL check, in order: the signature, the protocol and instance, the sender's role binding, that the role may send the message type in the instance's current state, and the payload shape. The verdict SHALL be accepted, or refused with a reason from a fixed list. The function SHALL run unchanged for a message that came over any transport.

#### Scenario: Message out of state refused
- **WHEN** `seller` sends `quote` before any `quote.request`
- **THEN** the verdict is refused with the reason `not enabled in state`

#### Scenario: Wrong role refused
- **WHEN** `buyer` sends a message that only `seller` may send in the current state
- **THEN** the verdict is refused with the reason `role not permitted`

#### Scenario: Payload mismatch refused
- **WHEN** a payload fails the declared shape
- **THEN** the verdict is refused with the reason `payload shape`

### Requirement: Causal Delivery
A receiver SHALL apply a message to its checker state only after every message listed in its dependencies has been applied. It SHALL hold a message with missing dependencies in a buffer whose size and timeout are declared in the protocol document. When the timeout passes, the receiver SHALL take the chart's cancel path and SHALL report the stall.

#### Scenario: Reordered messages applied in order
- **WHEN** a reply arrives before the request it depends on, and then the request arrives
- **THEN** both are applied, request first

#### Scenario: Timeout reports a stall
- **WHEN** the dependency never arrives within the declared timeout
- **THEN** the message is dropped from the buffer and the stall is reported

#### Scenario: Buffer is bounded
- **WHEN** held messages exceed the declared buffer size
- **THEN** the oldest is refused with the reason `buffer full`

### Requirement: Violation Records
A refused message SHALL be recorded with its reason and the signed message itself, so any participant can re-run the check and reach the same verdict. A receiver MAY send a signed `violation` notice to the instance's participants. Two validly signed different messages from one role with the same sequence number SHALL be reported as equivocation, with both messages as the proof.

#### Scenario: Violation proof verifies
- **WHEN** a participant receives a recorded violation with its signed message
- **THEN** re-running `accept` on its own state of the instance gives the same refusal

#### Scenario: Equivocation proof
- **WHEN** one role signs two different messages with the same sequence number
- **THEN** the pair is reported as equivocation and verifies for any third party

### Requirement: Test Vectors
The crate SHALL publish test vectors as JSON files: canonical forms and identities of documents, projections, envelope hashes and signatures, and verdict sequences with their expected outcomes. The Rust implementation SHALL pass every vector, and the files SHALL be usable without Rust.

#### Scenario: Vectors pass in Rust
- **WHEN** the vector suite runs
- **THEN** every identity, projection, hash and verdict equals the vector

#### Scenario: Vectors consumable elsewhere
- **WHEN** a TypeScript test reads the same files
- **THEN** it can compute the identities and verdicts with no Rust code

### Requirement: Lean Protocol Model
The Lean package SHALL model projection and `accept` and prove, as invariants of every reachable state:
- an accepted message is enabled in the role's projection;
- a role not bound in an open instance is never accepted;
- applying messages in any causally consistent order reaches the same state.

A Lean checker `vertep-kolyada` SHALL replay recorded JSONL histories in strict mode and report the first failing step. An unsafe variant that skips the role check SHALL have a counterexample.

#### Scenario: Theorems proven
- **WHEN** the Lean package is built
- **THEN** the three theorems are checked

#### Scenario: Unsafe variant rejected
- **WHEN** the unsafe variant is analysed
- **THEN** a reachable state accepts a message from an unbound role

#### Scenario: Rust history replays
- **WHEN** the Rust tests record the history of a protocol run and the checker replays it
- **THEN** the replay accepts it, and a doctored history is rejected at its first bad step
