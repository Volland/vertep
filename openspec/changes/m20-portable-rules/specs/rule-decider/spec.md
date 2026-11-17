## MODIFIED Requirements

### Requirement: Bundle Includes Rule Closure
An explanation bundle for a rule decision SHALL include the program closure and the engine family and version, and `vt verify-bundle` MUST re-run the rule offline and compare its conclusion and support with the journal. For a program inside the portable profile, verification MAY run on the other engine than the one that decided, and the report MUST say `cross-engine` when it does.

#### Scenario: Offline re-run
- **WHEN** a bundle is verified on a host with no network and no model
- **THEN** the rule re-runs and the conclusion and support match

#### Scenario: Tampered program
- **WHEN** the program source in a bundle is altered
- **THEN** verification fails on the closure digest

#### Scenario: Cross-engine verification
- **WHEN** a bundle decided by the native engine for a portable program is verified on a portable-only build
- **THEN** it verifies and the report says `cross-engine`
