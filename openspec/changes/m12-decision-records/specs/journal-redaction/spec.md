## Purpose

Lets personal data be removed from an append-only journal without invalidating it, and says honestly what replay can and cannot reproduce afterwards, so that regulated deployments can meet erasure rules without keeping personal data out of events by convention alone.

## ADDED Requirements

### Requirement: Sensitive Fields
The type language SHALL let a chart mark event and datamodel fields sensitive, with the marker present in the JSON Schema and SHACL projections. A sensitive field's value in a journal entry MUST be sealed under a per-subject data key, and the entry hash MUST cover the ciphertext.

#### Scenario: A marked field is sealed
- **WHEN** an event with a field marked sensitive is journaled
- **THEN** the entry holds the field as ciphertext and a key reference, and no plaintext of it is in storage

#### Scenario: An authorised reader sees it
- **WHEN** a reader with access to the subject's key opens the journal
- **THEN** the field is readable and replay is unchanged

#### Scenario: Marking is projected
- **WHEN** the chart's schema is projected to JSON Schema and SHACL
- **THEN** both name the field as sensitive

### Requirement: Erasure By Key Destruction
The system SHALL erase a subject's data by destroying the subject's key through a `KeyStore` trait and appending an `erasure` entry that names the subject and the time. The journal MUST remain valid: every hash and signature checks after erasure. An erasure MUST itself be journaled, and a key store that cannot guarantee destruction MUST NOT be accepted as conforming.

#### Scenario: Erased data is unreadable
- **WHEN** a subject is erased
- **THEN** their sealed fields cannot be read by anyone, and the journal's chain verifies

#### Scenario: Erasure is recorded
- **WHEN** a subject is erased
- **THEN** an erasure entry names the subject and the time and is part of the chain

#### Scenario: Other subjects are unaffected
- **WHEN** one subject is erased
- **THEN** another subject's fields in the same journal remain readable

### Requirement: Replay After Erasure
Replay of a journal containing erased fields SHALL follow the transitions the entries recorded for any guard that reads an erased field, and MUST mark the result `replay: recorded` instead of `replay: verified`. The explanation view and bundle verification MUST show the marker and MUST NOT present such a replay as full verification.

#### Scenario: Replay completes with a marker
- **WHEN** an actor whose guard reads an erased field is replayed
- **THEN** the replay reaches the recorded states and the result is marked `recorded`

#### Scenario: Unaffected journals stay verified
- **WHEN** a journal with no erased field a guard reads is replayed
- **THEN** the result is `verified`

### Requirement: Retention Policy
A chart type SHALL be able to declare a retention horizon for sensitive fields. When the horizon passes, the system MUST erase the affected fields as an erasure, journaled with the reason `retention`.

#### Scenario: A horizon expires
- **WHEN** a field's retention horizon passes
- **THEN** its subject's data is erased and an erasure entry with reason `retention` is appended

#### Scenario: No policy, no expiry
- **WHEN** a chart type declares no retention horizon
- **THEN** nothing expires
