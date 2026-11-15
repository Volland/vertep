## Purpose

Limits stated once as rule programs and enforced at the commit of every journal append in a shard, so a violating transition cannot be recorded, and so the refusal is visible.

## ADDED Requirements

### Requirement: Enforced Program Refuses A Violating Append
When a rule program is enforced, a journal append whose transaction would make the program report a violation SHALL be refused, and the actor's journal, state and head MUST be unchanged.

#### Scenario: Approval without review
- **WHEN** an append would record `approve` for a case with no officer-review statement and the invariant requires one
- **THEN** the append is refused and the journal has no new entry

#### Scenario: A conforming append
- **WHEN** an append violates no enforced program
- **THEN** it commits as before

### Requirement: Refusal Is Visible And Writes No Entry
A refused append SHALL be reported to the sender and recorded in the dead-letter set with the violation name, message and offending statements. It MUST NOT write an entry.

#### Scenario: Dead letter
- **WHEN** an append is refused
- **THEN** the dead-letter set holds the message with the violation name, the message text and the offending statement ids

#### Scenario: No entry
- **WHEN** the journal is read after a refusal
- **THEN** its entries and head are those from before the append

### Requirement: Enabling Checks Existing Data
Turning enforcement on, or changing an enforced program's text, SHALL run the full check over the shard's current data, and MUST be refused with the violations listed if live data violates it.

#### Scenario: Live data violates
- **WHEN** enforcement is requested for a program that existing statements violate
- **THEN** it is refused and the violations are listed

### Requirement: Delta Check Agrees With Full Check
For every enforced program, the commit-time delta check SHALL refuse a transaction if and only if the full check on the resulting data has a violation the data before did not.

#### Scenario: Random transactions
- **WHEN** random transactions are applied to a random journal with an enforced program
- **THEN** each refused transaction adds a violation to the full check, and each accepted one adds none
