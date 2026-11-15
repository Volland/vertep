# rule-authoring Specification

## Purpose
Lets an agent propose rule programs, gates them, runs them in shadow, and promotes them by recorded acts, while making it impossible for an actor to change the rules that govern itself.

## Requirements

### Requirement: Proposals Pass A Gate
A proposed rule program SHALL be stored in state `proposed` and MUST NOT be read by any decider or enforced until it passes, in order, the capability check for the proposing identity, the type check against the chart's event types, the profile check, a trial run on recorded cases under fuel and, for invariants, a check against existing data.

#### Scenario: A passing proposal
- **WHEN** a proposal passes every step
- **THEN** it is stored as `proposed` and listed with its gate results

#### Scenario: A proposal is inert
- **WHEN** a proposal exists
- **THEN** no decision and no append is affected by it

### Requirement: Gate Errors Are Structured
A failing proposal SHALL be rejected with an error carrying a stable code, the offending clause and the reason, and the first failing step MUST stop the gate.

#### Scenario: A type error
- **WHEN** a clause concludes an event the chart does not declare
- **THEN** the error code is `type-error` and names the clause and the event

#### Scenario: Repair
- **WHEN** the proposer resubmits a program corrected from the error
- **THEN** the gate evaluates the new program from the start

### Requirement: Authorship Is Recorded
A program node SHALL record the proposing identity, the grant used and the source of the proposal, and each version MUST keep its own record.

#### Scenario: Who proposed it
- **WHEN** a program is read
- **THEN** its proposer, grant and source are available for every version

### Requirement: Shadow Evaluation
A proposed program SHALL be evaluable in shadow beside the live program on real offers, recording each difference between its conclusion and the live decision, and MUST NOT affect any decision or append while in shadow.

#### Scenario: Shadow difference
- **WHEN** the proposal would have refused a case the live program accepted
- **THEN** the difference is recorded with both supports and the decision is the live one

### Requirement: Promotion Is Staged And Journaled
Promotion SHALL proceed from shadow to live for a named scope to live, each step a journaled act by an identity holding the promote grant, and MUST NOT skip a step.

#### Scenario: A skipped step
- **WHEN** promotion from `proposed` straight to live is requested
- **THEN** it is refused and names the missing step

#### Scenario: A step is journaled
- **WHEN** a program moves to live for a scope
- **THEN** a journal entry records who, when, the scope and the program digest

### Requirement: No Self-Governance
The system MUST refuse to propose, edit or promote a rule program that would govern the decisions or appends of the actor, or of an actor controlled by the identity, that makes the request, whatever grants it holds.

#### Scenario: An agent edits its own rules
- **WHEN** an agent proposes a program whose scope includes its own address
- **THEN** the proposal is refused with code `self-governance`

#### Scenario: A reviewer's grant
- **WHEN** a different identity holding the promote grant promotes a program that governs the agent
- **THEN** the promotion is allowed

### Requirement: Rule Diff
The system SHALL compare two versions of a program and report the clause changes and the decisions on recorded cases that would differ.

#### Scenario: A readable diff
- **WHEN** two versions are compared
- **THEN** the report lists added, removed and changed clauses and the recorded decisions that differ
