## Purpose

Makes the journal of a decision-making actor sufficient to explain, verify and re-examine every decision: what state the process was in, what was offered, who chose and whether the answer was accepted, under which version of the rules, and what followed, exportable so that a third party can check it offline.

## ADDED Requirements

### Requirement: Decision Disposition Is Recorded
A decision journal entry, and the entry of a decider's refusal, SHALL record a disposition, one of `accepted`, `refused-not-offered`, `refused-guard`, `refused-invalid`, `stale`, `fallback-on-deadline`, `fallback-on-refusal` or `refused-by-decider`. The shell MUST set it from the validation it performed before it journaled the input, and MUST NOT derive it later from the next state. `refused-invalid` is an answer whose annotations or payload fail validation; `stale` is an answer with no open offer; `refused-guard` is an offered event whose transition was no longer enabled; `fallback-on-refusal` is reserved for a chart path that consults a fallback after a refusal, which does not exist yet. Entries written before this requirement exist MUST read as `unknown`.

#### Scenario: A refused answer is marked refused
- **WHEN** a decider answers with an event that was not among the offered candidates
- **THEN** the decision entry has disposition `refused-not-offered`
- **AND** the state as of that entry is the one the chart's `error.decide.invalid` transition reaches

#### Scenario: An accepted answer is marked accepted
- **WHEN** a decider answers with an offered, enabled event
- **THEN** the decision entry has disposition `accepted`

#### Scenario: A deadline fallback is marked
- **WHEN** no answer arrives before the decide state's deadline and the fallback decider answers
- **THEN** the fallback's decision entry has disposition `fallback-on-deadline`

#### Scenario: An old journal opens
- **WHEN** a journal written before dispositions existed is opened
- **THEN** its decision entries read as `unknown` and replay is unchanged

### Requirement: Offer Is Part Of The Record
A decision entry SHALL record the candidate events that were enabled and offered, in offer order, as of the offer. A reader MUST NOT need the chart or the datamodel to know what was offered.

#### Scenario: The offer is on the entry
- **WHEN** a decider is asked in a state whose guard makes `approve` false
- **THEN** the decision entry's offer lists the enabled candidates and does not list `approve`

#### Scenario: No decision, no offer
- **WHEN** the chart takes a transition by rule and consults no decider
- **THEN** no decision entry exists for that step, and the explanation view says that no decider was asked

### Requirement: Explanation View
The system SHALL provide `explain(actor, position)`, derived from the journal and chart versions by replay and never stored, returning the state configuration, the offer, the chosen event, the decider's identity, the disposition, the chart version, the transitions taken and the effects requested. It MUST be available from Rust, the command bridge, JavaScript and Python.

#### Scenario: Explain a referred loan
- **WHEN** an auditor calls `explain` at the position of a refused underwriter answer
- **THEN** the result lists the offer, the refused event, the refusal disposition and the officer state that followed

#### Scenario: Explain needs no decider
- **WHEN** `explain` is called on a host with no decider bound
- **THEN** it returns the same result as on the host that ran the decision

#### Scenario: Explain is derived
- **WHEN** the journal is altered and `explain` is called again
- **THEN** the result changes with it, because nothing is cached outside the journal

### Requirement: Rationale Is Labelled As A Claim
The explanation view SHALL present a decider's rationale as text claimed by that decider and MUST label it unverified in every output format and in the JSON schema. It MUST NOT merge the rationale into a field presented as the reason for the transition.

#### Scenario: The label is in the output
- **WHEN** a decision with a rationale is explained
- **THEN** the rationale appears under the decider's identity with an unverified marker, separate from the transition taken

### Requirement: Explanation Bundle
The system SHALL export a bundle for one actor and a range of positions: the journal slice, the chart versions it uses addressed by hash, and a manifest with digests. `vt verify-bundle` MUST verify it offline, with no decider, no network and no runtime, by recomputing the explanation view and comparing it with the manifest, and MUST fail on any altered entry, missing chart or wrong digest.

#### Scenario: A bundle verifies offline
- **WHEN** a bundle is exported and verified on a machine without the original runtime
- **THEN** verification succeeds and no decider is called

#### Scenario: An altered entry is detected
- **WHEN** one entry of a bundle is changed
- **THEN** verification fails and names the entry

#### Scenario: A missing chart is detected
- **WHEN** the chart a bundle's entries cite is absent from it
- **THEN** verification fails and names the chart's hash

### Requirement: Agent Journal Integrity
An agent actor type SHALL be able to opt in to a hash-chained, signed journal using the envelope of `vertep-objects`. A verifier MUST accept chains written by either. An actor type that does not opt in MUST behave as before.

#### Scenario: A chained journal verifies
- **WHEN** an opted-in actor decides and its journal is verified
- **THEN** every entry's hash and signature check and the chain is unbroken

#### Scenario: A reordered entry is detected
- **WHEN** two entries of a chained journal are swapped
- **THEN** verification fails at the first affected entry

#### Scenario: Promotion keeps history
- **WHEN** an opted-in agent actor is promoted to a durable object
- **THEN** its existing entries verify unchanged under the object's verifier

### Requirement: Counterfactual Replay
The system SHALL replay a journal's inputs under another chart version into a separate scratch journal, report every position where the configuration diverges, and MUST NOT write to the original journal. Decisions MUST be taken from the recorded ones or from a supplied decider, as the caller chooses, and the report MUST say which.

#### Scenario: A new ceiling changes an outcome
- **WHEN** the loan chart's amount ceiling is lowered and last quarter's applications are replayed under it with recorded decisions
- **THEN** the report lists each application whose recorded `approve` would now be refused, with its position

#### Scenario: The original is untouched
- **WHEN** a counterfactual replay completes
- **THEN** the original journal's entries and digests are identical to before
