# production-readiness Specification

## Purpose
Gives an adopter evidence and a clear boundary: which parts are mature, what they cost, how to build them from a clean checkout, what supply-chain controls surround them, and that the documentation says what the code does.

## Requirements

### Requirement: Stability Tiers
Every published crate and npm package SHALL declare a tier of `stable`, `preview` or `experimental` in its manifest. The README and the site MUST render their maturity tables from those declarations, and CI MUST fail when a rendered table differs from the manifests.

#### Scenario: A tier is declared
- **WHEN** a manifest of a published crate has no tier
- **THEN** the check fails and names the crate

#### Scenario: A table cannot drift
- **WHEN** a manifest's tier changes without the table being regenerated
- **THEN** CI fails

### Requirement: Reproducible Toolchain
The repository SHALL build `@vertep/web` and the Python wheel from a clean checkout with one documented command, inside a container whose toolchain versions are pinned, and CI MUST run that build. No step may depend on a maintainer's machine configuration.

#### Scenario: Clean build
- **WHEN** CI checks out the repository on a clean runner and runs the documented command
- **THEN** both artifacts are produced and their smoke tests pass

#### Scenario: A version drift is caught
- **WHEN** the pinned `wasm-bindgen` version and the library's differ
- **THEN** the build fails with a message naming both

### Requirement: Supply Chain Gate
CI SHALL fail on a dependency with a known advisory or a disallowed licence, every release MUST publish an SBOM, and release artifacts MUST be signed with provenance a consumer can verify.

#### Scenario: An advisory blocks a release
- **WHEN** a dependency has an open advisory
- **THEN** CI fails before a release can be tagged

#### Scenario: Artifacts are verifiable
- **WHEN** a consumer verifies a released crate, package or wheel
- **THEN** its signature and provenance check against the release workflow

### Requirement: Docs Match Behaviour
Every statement on the site or in `tg.md/roadmap.md` that a capability is delivered or not yet available SHALL agree with the code, and an audit run before each release MUST list any disagreement.

#### Scenario: The agent journal statement is reconciled
- **WHEN** the audit compares the decider-agent page's warning about the agent journal with the delivered durable agent journal
- **THEN** the page and the roadmap state the same thing, and the statement is true of the code

#### Scenario: An audit finds a stale claim
- **WHEN** a page says a feature is missing that the code provides
- **THEN** the audit reports the page and the code location

### Requirement: Ten Minute Path
The site's quick start and the `credit-decision` example SHALL be run by CI on a clean runner, and CI MUST fail when either does not complete or does not produce its recorded output.

#### Scenario: The quick start runs
- **WHEN** CI follows the documented install and first run
- **THEN** the first example completes with its recorded output

#### Scenario: A broken quick start fails CI
- **WHEN** a change breaks a documented step
- **THEN** CI fails and names the step
