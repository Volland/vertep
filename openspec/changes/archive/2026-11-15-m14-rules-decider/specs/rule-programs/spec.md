## Purpose

Rule programs are versioned data that live beside the journal they decide for, run under a bounded budget, and are pinned by every decision that used them, so that any decision can be re-run against the rules in force when it was taken.

## ADDED Requirements

### Requirement: Rule Programs Live In The Shard
A rule program SHALL be stored as a node in the same tiramemsu file as the journal of the actors it decides for, with its source as one statement, and editing it SHALL supersede that statement. A program MUST be readable as of any earlier transaction.

#### Scenario: A program is stored with the journal
- **WHEN** a program is put into a shard
- **THEN** it is written by one transaction in the shard's file and no other file changes

#### Scenario: An edit is a supersede
- **WHEN** a program is edited
- **THEN** the earlier source remains readable at the earlier transaction and the new source is live

#### Scenario: A program includes another
- **WHEN** a program accumulates another stored program
- **THEN** its closure lists both and a cycle is refused with a typed error

### Requirement: Rule Version Is Pinned By The Entry
A decision entry produced by a rule program SHALL record the digest of the program closure, the engine version and the fuel limit. Replay MUST use that closure as of the entry's transaction, and MUST report a difference in engine version.

#### Scenario: Replay uses the rules of the day
- **WHEN** a program is edited after a decision and the decision is replayed
- **THEN** the replay runs the closure recorded on the entry and reaches the same decision

#### Scenario: Engine version differs
- **WHEN** a journal is replayed with a different engine version
- **THEN** replay reports the difference and, in strict mode, fails

### Requirement: Evaluation Is Bounded
Every rule evaluation SHALL run under a step budget set by policy and MUST NOT depend on wall-clock time. Exhausting the budget SHALL produce a typed error, not a partial answer.

#### Scenario: A runaway rule ends
- **WHEN** a program loops
- **THEN** the run ends with a `fuel-exhausted` error naming the budget, and the decider returns a refusal

#### Scenario: Fuel used is recorded
- **WHEN** a decision completes
- **THEN** the entry records the fuel used and the same run on replay uses the same fuel

### Requirement: Native Library Is Optional
Rule evaluation SHALL require the native engine library behind a cargo feature that is off by default. On a build without it, loading a chart that declares a rules decider MUST fail with a typed `rules-unavailable` error and MUST NOT fall back to another decider.

#### Scenario: Browser build
- **WHEN** a chart with a rules decider is loaded in the WebAssembly build
- **THEN** loading fails with `rules-unavailable`

#### Scenario: Feature off
- **WHEN** the workspace is built with default features
- **THEN** it builds and links no native engine library
