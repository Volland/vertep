## Purpose

A decider whose answer is a derivation over recorded facts: the offer and the actor's state are statements, a stored rule program concludes the decision and states what it rests on, and the record keeps that support so the decision re-runs offline.

## ADDED Requirements

### Requirement: Rule Decider
The system SHALL provide a decider of kind `Rules` that evaluates a rule program over the offer and returns a decision or a refusal under the same contract as every other decider. It MUST NOT return an event that was not offered.

#### Scenario: A rule chooses an offered event
- **WHEN** a decide state offers `approve` and `refer` and the program concludes `refer`
- **THEN** the decision is `refer` with disposition `accepted`

#### Scenario: A rule names an event that was not offered
- **WHEN** the program concludes `approve` and only `refer` and `decline` were offered
- **THEN** the decider returns a refusal and the disposition is `refused-not-offered`

### Requirement: Derivation Is The Rationale
A decision from a rule program SHALL carry the statements the conclusion states as its support, and the explanation view MUST label this a derivation, distinct from a claim. The host MUST refuse a support that names a statement the run did not read.

#### Scenario: Support is listed
- **WHEN** a decision is explained
- **THEN** the view lists the supporting statements and the program that concluded, labelled `derivation`

#### Scenario: Forged support
- **WHEN** a program states a support statement that its run never read
- **THEN** the run fails and the decider returns a refusal

### Requirement: Offer And State Are Statements
Before a rule runs, the shell SHALL write the offered candidates and the actor's context as statements in the entry's graph, in offer order, so a program and its support can refer to them. This MUST be the same record the explanation view reads as the offer.

#### Scenario: The offer is readable by the rule
- **WHEN** a decision is requested with two candidates
- **THEN** both are statements in the entry's graph in offer order before the program runs

#### Scenario: One record
- **WHEN** the explanation view reports the offer
- **THEN** it reads the same statements the rule read

### Requirement: No Applicable Rule Is A Refusal
If no rule applies, the program has no solution, or fuel is exhausted, the decider SHALL return a refusal with a reason, and MUST NOT choose a default. A chart that wants a default declares a fallback decider.

#### Scenario: Nothing applies
- **WHEN** no rule concludes for the offered candidates
- **THEN** the decider refuses with reason `no-applicable-rule` and the chart's fallback, if any, is consulted

### Requirement: Bundle Includes Rule Closure
An explanation bundle for a rule decision SHALL include the program closure and the engine version, and `vt verify-bundle` MUST re-run the rule offline and compare its conclusion and support with the journal.

#### Scenario: Offline re-run
- **WHEN** a bundle is verified on a host with no network and no model
- **THEN** the rule re-runs and the conclusion and support match

#### Scenario: Tampered program
- **WHEN** the program source in a bundle is altered
- **THEN** verification fails on the closure digest

### Requirement: Rule Version Counterfactual
Counterfactual replay SHALL accept a different rule program version and report each decision that would differ, without changing the original journal.

#### Scenario: New policy
- **WHEN** a journal is replayed under the next version of a program
- **THEN** the report lists the decisions that differ with their old and new support, and the original journal is unchanged
