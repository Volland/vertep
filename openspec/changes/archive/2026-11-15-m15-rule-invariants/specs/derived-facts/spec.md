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
