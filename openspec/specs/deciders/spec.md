# deciders Specification

## Purpose
Deciders let something outside the chart's guards (an LLM, a human, another actor, or a deterministic policy) choose the next move at a declared choice point. The chart still bounds the action space, every decision is a journaled event, and a run stays replayable and auditable.

## Requirements

### Requirement: Decide State Declaration
The system SHALL support a `vt:decide` state kind, declared in every authoring surface. A decide state is an atomic or compound state that names exactly one decider binding and, optionally, a deadline, a fallback decider, and a set of event descriptors that are offered as candidates.

#### Scenario: Decide state parsed from SCXML
- **WHEN** an SCXML document contains a `<state>` carrying the `vt:decide` extension attribute with a decider binding
- **THEN** the IR contains a decide state with that binding, and the chart's RDF projection types the state as `vt:DecideState`

#### Scenario: Decide state without decider rejected
- **WHEN** a chart declares a `vt:decide` state that names no decider binding
- **THEN** chart validation fails with an error identifying the state, and the chart is not accepted for execution

#### Scenario: Unknown decider binding rejected at instantiation
- **WHEN** a chart is instantiated and its decide state names a decider binding that is not registered in the runtime
- **THEN** instantiation fails with an error naming the missing binding, and no actor is activated

### Requirement: Enabled Event Offer
On entering a decide state, the system SHALL compute the candidate set. The candidate set is every event descriptor for which a transition would be enabled from the current configuration under the current datamodel, restricted to the state's declared candidate descriptors when given. The system SHALL hand the candidate set, with each candidate's payload schema, to the decider.

#### Scenario: Only enabled transitions are offered
- **WHEN** a decide state has outgoing transitions on `approve`, `reject` and `escalate`, and the guard of `escalate` is false in the current context
- **THEN** the candidate set handed to the decider contains `approve` and `reject` and does not contain `escalate`

#### Scenario: Candidates include enabled transitions of ancestors
- **WHEN** a decide state is nested in a compound state whose transition on `cancel` is enabled
- **THEN** `cancel` is part of the candidate set, because SCXML transition selection would take it

#### Scenario: Empty candidate set
- **WHEN** a decide state is entered and no candidate transition is enabled
- **THEN** the decider is not consulted, and the internal event `vt.decide.none` is raised with the decide state's id

### Requirement: Decider Contract
The system SHALL define a decider contract that receives a decision request and eventually yields either one decision or a refusal. A decision request contains the decide state id, the candidate set with payload schemas, a read-only view of the context, the actor IRI and a correlation id. A decision contains one chosen event name, a payload, and optional annotations. A decider SHALL NOT mutate the chart, the context or the store.

#### Scenario: Decider returns a decision
- **WHEN** a decider is asked to decide with candidates `approve` and `reject` and returns `approve` with a valid payload
- **THEN** the runtime delivers a `vt.decision` input to the actor that carries `approve`, the payload and the correlation id

#### Scenario: Decider refuses
- **WHEN** a decider returns a refusal with a reason
- **THEN** the internal event `vt.decide.refused` is raised in the decide state with the reason, and no candidate transition is taken

#### Scenario: Decider has no write access
- **WHEN** a decider implementation attempts to change the context it was given
- **THEN** the change is not visible to the chart, because the decider receives an immutable view

### Requirement: Decisions Are Journaled Inputs
Every decision SHALL reach the actor as an external input event that is appended to the actor's journal before it is processed. Replay SHALL reuse the journaled decision and SHALL NOT consult the decider again.

#### Scenario: Decision appended before processing
- **WHEN** a decider returns a decision for a virtual actor
- **THEN** the journal contains an entry for the decision event, with its correlation id and annotations, before the transition it triggers is executed

#### Scenario: Replay does not call the decider
- **WHEN** an actor whose journal contains a decision is replayed from its journal
- **THEN** the actor reaches the same configuration and context, and the decider is invoked zero times

#### Scenario: Duplicate decision deduplicated
- **WHEN** the same decision, with the same correlation id, is delivered twice
- **THEN** the second delivery is acknowledged and dropped, and the actor processes the decision exactly once

### Requirement: Decision Validation Against Candidates
The system SHALL accept a decision only if its chosen event is in the candidate set that was offered with the same correlation id, and its payload conforms to that candidate's payload schema. Any other decision SHALL be rejected with the internal error event `error.decide.invalid`, and no transition SHALL be taken because of it.

#### Scenario: Decision outside the candidate set rejected
- **WHEN** a decider returns the event `delete_all`, which was not in the offered candidate set
- **THEN** the event `error.decide.invalid` is raised, with the reason `not-a-candidate`, and the configuration does not change because of the decision

#### Scenario: Payload that violates the schema rejected
- **WHEN** a decider returns `approve` with a payload missing a field required by `approve`'s payload schema
- **THEN** the event `error.decide.invalid` is raised, with the reason `payload-schema` and the violated path

#### Scenario: Stale decision rejected
- **WHEN** a decision arrives with a correlation id from an earlier visit of the decide state
- **THEN** the event `error.decide.invalid` is raised, with the reason `stale`, and the decision is not applied

### Requirement: Re-decision After Context Change
The system SHALL invalidate an outstanding decision request, and issue a new request with a new correlation id, when the decide state stays active and a later macrostep changes the candidate set.

#### Scenario: Candidate set shrinks while waiting
- **WHEN** a decision request offering `approve` and `reject` is outstanding, and an external event changes the context so that `approve` is no longer enabled
- **THEN** the outstanding request is cancelled, a new request offering only `reject` is issued with a new correlation id, and a late `approve` decision is rejected as stale

#### Scenario: Unchanged candidate set keeps the request
- **WHEN** a macrostep runs while a request is outstanding, and the candidate set and its schemas do not change
- **THEN** the outstanding request and its correlation id stay valid

### Requirement: Decision Deadline and Fallback
A decide state SHALL support an optional deadline and an optional fallback decider. When the deadline passes without an accepted decision, the system SHALL consult the fallback decider with the same candidate set, or raise `error.decide.timeout` when there is no fallback. Deadlines SHALL be durable timers, so they survive deactivation and replay.

#### Scenario: Deadline falls back to a deterministic decider
- **WHEN** an LLM decider does not answer within the declared deadline of 30 seconds, and a guard-based fallback decider is declared
- **THEN** the fallback decider is consulted with a new correlation id, and its decision is journaled with the annotation that a fallback was used

#### Scenario: Deadline without fallback
- **WHEN** the deadline passes and no fallback decider is declared
- **THEN** the event `error.decide.timeout` is raised in the decide state, and late decisions for that request are rejected as stale

#### Scenario: Deadline survives deactivation
- **WHEN** a virtual actor in a decide state with a pending deadline is deactivated, and the deadline passes
- **THEN** the actor is reactivated by the timer and the deadline handling runs as if the actor had stayed active

### Requirement: Decider Kinds
The system SHALL provide four decider kinds behind the same contract:
- a **guard decider**, which picks the first candidate in document order whose declared decision guard holds
- an **LLM decider**, which asks a model for structured output restricted to the candidate set
- a **human decider**, which publishes the request to an inbox and waits for a person's answer
- an **actor decider**, which sends the request to another actor and accepts its reply

#### Scenario: Guard decider is deterministic
- **WHEN** a guard decider is consulted twice with the same candidates and context
- **THEN** it returns the same decision both times

#### Scenario: Human in the loop
- **WHEN** a decide state bound to a human decider is entered
- **THEN** the request appears in the configured human inbox with the candidates and their payload schemas, and the person's answer is delivered as a journaled decision

#### Scenario: Actor decider uses the request protocol
- **WHEN** a decide state is bound to an actor decider that names a virtual actor IRI
- **THEN** the request is sent as a non-blocking `vt:request` with the decision correlation id, and the reply is validated like any other decision

### Requirement: Typed Decision Schemas
When a chart is typed, the system SHALL derive each candidate's payload schema from the event's declared payload type in the IR type language. The schema SHALL be offered to deciders as JSON Schema. When a chart is untyped, each candidate's payload schema SHALL be the schema of the dynamic value type.

#### Scenario: JSON Schema from typed events
- **WHEN** a typed chart declares `approve` with the payload record `{ amount: decimal, reason: string }`
- **THEN** the candidate `approve` carries a JSON Schema requiring `amount` as a number and `reason` as a string

#### Scenario: Untyped chart still decidable
- **WHEN** a decide state belongs to a chart with no type declarations
- **THEN** each candidate carries the dynamic value schema, and decisions are validated only for the event name

### Requirement: Decision Annotations
The system SHALL let a decision carry annotations: rationale, confidence between 0 and 1, decider kind, decider identity, and, for LLM deciders, model identifier and prompt hash. In the graph, annotations SHALL be recorded as RDF 1.2 triple-term annotations on the statement that links the decision to the chosen event. Annotations SHALL NOT influence transition selection.

#### Scenario: Annotated LLM decision in the graph
- **WHEN** an LLM decider returns `approve` with rationale "invoice matches PO" and confidence 0.82
- **THEN** the journal graph contains the statement linking the decision to `approve`, annotated with `vt-agent:rationale`, `vt-agent:confidence` 0.82, the model identifier and the prompt hash

#### Scenario: Annotations do not change behaviour
- **WHEN** two decisions choose the same event with the same payload, but carry different rationales and confidences
- **THEN** the resulting configuration and context are identical

#### Scenario: Confidence out of range rejected
- **WHEN** a decision carries confidence 1.4
- **THEN** the decision is rejected with `error.decide.invalid` and the reason `annotation`

### Requirement: Non-blocking Decision Wait
While a decision request is outstanding, the decide state SHALL remain an ordinary active state. The actor SHALL keep processing other events, and transitions that leave the decide state SHALL cancel the outstanding request.

#### Scenario: Other events processed while waiting
- **WHEN** a decision request is outstanding and an unrelated event `ping` arrives with an enabled transition in a parallel region
- **THEN** `ping` is processed immediately, and the decision request stays outstanding

#### Scenario: Leaving the decide state cancels the request
- **WHEN** an external `cancel` event moves the actor out of the decide state while a request is outstanding
- **THEN** the request is cancelled, the decider is notified of the cancellation, and a later decision for it is rejected as stale

### Requirement: Decider Safety
For every decider kind and every decider output, the transition the system takes because of a decision SHALL be a transition that was enabled in the configuration and context at the time the candidate set was offered. This property SHALL be stated and proven in the formal model.

#### Scenario: Arbitrary decider output is safe
- **WHEN** the differential test harness feeds randomly generated decider outputs, including unknown events, malformed payloads and stale correlation ids, to randomly generated charts with decide states
- **THEN** every transition taken is in the enabled set recorded at offer time, and the Rust core and the Lean oracle produce identical traces

#### Scenario: Safety theorem checked in CI
- **WHEN** the formal model is built in CI
- **THEN** the decider safety theorem is checked without `sorry` or new axioms
