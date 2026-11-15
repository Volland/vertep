## Purpose

Facts that a rule program concludes and the store keeps, each with the statements it follows from, so that they can be explained, retract with their cause, and obey erasure.

## ADDED Requirements

### Requirement: Derived Facts Carry Support
A fact materialized by a rule program SHALL be written into the graph of the actor it concerns with its support statements and the program closure it came from, and the explanation view MUST be able to reach it from its support.

#### Scenario: A risk flag with its cause
- **WHEN** a program concludes a `high-risk` fact from two statements
- **THEN** the fact is in the actor's graph and `dependents` of either statement lists it

### Requirement: Derived Facts Retract With Support
A derived fact SHALL be retracted when any statement in its support is retracted or when the program that concluded it changes so that it no longer concludes it.

#### Scenario: Support retracted
- **WHEN** a supporting statement is retracted
- **THEN** the derived fact is no longer live and its retraction is visible in the history

#### Scenario: Program edited
- **WHEN** the program is edited so that it no longer concludes the fact
- **THEN** reconciliation retracts the fact

### Requirement: Erasure Retracts Dependents
When a subject's key is destroyed under `journal-redaction`, derived facts whose support includes that subject's sealed statements SHALL be retracted, and the retraction MUST be journaled with the erasure. Derived facts that do not depend on them MUST remain.

#### Scenario: Erasing a subject
- **WHEN** a subject's key is destroyed
- **THEN** derived facts that rest on their sealed statements are retracted and the erasure entry lists them

#### Scenario: Unrelated facts survive
- **WHEN** a derived fact rests only on other subjects' statements
- **THEN** it remains live
