## Purpose

Makes the derivation behind a rule decision something a third party can check, and states plainly which programs are inside the checked subset and which are not.

## ADDED Requirements

### Requirement: Rules Profile
The system SHALL define a rules profile, a subset of the rule language for which totality, support necessity and certificates are defined, and SHALL mark every decision from a program outside it as `unchecked` in the explanation view. A program outside the profile MUST still run.

#### Scenario: Out of profile
- **WHEN** a program uses a construct outside the profile
- **THEN** it runs and its decisions are marked `unchecked`

#### Scenario: In profile
- **WHEN** a program is inside the profile
- **THEN** `vt rules verify` reports the checks that apply to it

### Requirement: Totality Check
For a program inside the profile and a decide state of a chart, the system SHALL decide whether every offer of that state yields a decision or an explicit refusal, and SHALL report a counterexample offer when it does not. When it cannot decide within its bound it MUST report `unknown`.

#### Scenario: A gap
- **WHEN** no clause covers an offer of `{refer}` alone
- **THEN** the check fails and lists that offer as the counterexample

#### Scenario: Covered
- **WHEN** every offer subset is covered by a clause or an explicit refusal
- **THEN** the check passes

### Requirement: Support Is Necessary
Offline verification of a bundle SHALL re-run each rule decision with each supporting statement removed in turn and MUST report a decision whose conclusion still holds as `support-not-necessary`.

#### Scenario: Padded support
- **WHEN** a program states a statement as support that its conclusion does not need
- **THEN** verification reports `support-not-necessary` for that statement

#### Scenario: Minimal support
- **WHEN** every supporting statement is needed
- **THEN** the check passes

### Requirement: Engines Agree On Horn Programs
For Horn programs inside the profile, the ELPI engine and the Datalog engine SHALL give the same conclusions on the same statements, and a disagreement MUST fail the conformance suite.

#### Scenario: Agreement on random data
- **WHEN** a Horn program is run on a pseudo-random graph in both engines
- **THEN** the conclusions are equal

### Requirement: Derivation Certificate
The system SHALL export a rule decision's derivation as an LF proof term and SHALL check it in the Lean model offline, with the program closure and engine version bound into the certificate.

#### Scenario: A valid certificate
- **WHEN** `vt certify` checks a bundle with a rule decision
- **THEN** the Lean checker accepts the derivation

#### Scenario: A forged conclusion
- **WHEN** the conclusion in a certificate is changed
- **THEN** the Lean checker rejects it

#### Scenario: Engine version mismatch
- **WHEN** the certificate's engine version differs from the journal's
- **THEN** certification fails with a version error
