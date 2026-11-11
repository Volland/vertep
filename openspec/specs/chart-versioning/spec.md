# chart-versioning Specification

## Purpose
Keeps long-lived event-sourced actors correct while their charts change: chart versions are content-addressed, every journal segment records the version it ran under, actors stay on their version until final, and upgrades happen only through explicit, journaled migrations.

## Requirements

### Requirement: Version Recorded Per Journal Segment
Every journal entry SHALL name the chart version IRI (the RDFC-1.0 content-address of the chart) under which its macrostep ran, and consecutive entries with the same version SHALL form a segment.

#### Scenario: Version on every entry
- **WHEN** an actor appends an entry
- **THEN** the entry names the version IRI of the chart that produced it

#### Scenario: Same chart from different surfaces
- **WHEN** the same chart is loaded once from SCXML and once from YAML
- **THEN** both loads have the same version IRI, and journals written by either are interchangeable

### Requirement: Replay With Recorded Version
Replay SHALL execute each journal segment with the chart version recorded for it, never with a newer version, and activation SHALL fail with a missing-version error if a recorded version cannot be loaded.

#### Scenario: Old segment replayed with old chart
- **WHEN** an actor's journal has a segment under v1 followed by a migration to v2
- **THEN** replay runs the v1 entries with v1, applies the migration, and runs later entries with v2

#### Scenario: Missing version
- **WHEN** a recorded version IRI is not available in the chart registry
- **THEN** activation fails with an error naming the version IRI and the actor is not activated

### Requirement: Chart Registry
The runtime SHALL keep every chart version that any journal references in a registry keyed by version IRI, and SHALL refuse to remove a version that a non-final actor still uses.

#### Scenario: Registering a new version
- **WHEN** an application registers a changed chart
- **THEN** the registry stores it under its new version IRI, alongside the old version

#### Scenario: Removal refused
- **WHEN** an operator removes a version still used by a non-final actor
- **THEN** the removal is refused and lists the actors that use it

### Requirement: Pin Until Final
By default an existing actor SHALL keep running the version it started with until it reaches a top-level final state, while new actors of the same chart type SHALL start with the latest registered version.

#### Scenario: Existing actor stays pinned
- **WHEN** v2 of chart type T is registered while actor A of type T runs v1
- **THEN** A keeps processing messages under v1

#### Scenario: New actor gets the latest
- **WHEN** a virtual actor of type T with an empty journal is activated after v2 is registered
- **THEN** it starts under v2

### Requirement: Explicit Migration
Moving a running actor to a new version SHALL require a declared migration from the old to the new version consisting of a total mapping from old configurations to new configurations and a datamodel transform, and the runtime SHALL record the migration in the journal as an entry holding the snapshot before, the migration's IRI and the snapshot after.

#### Scenario: Migration applied
- **WHEN** an operator migrates actor A from v1 to v2 with migration M
- **THEN** A's journal gains one migration entry with the v1 snapshot, M's IRI and the v2 snapshot, and later entries run under v2

#### Scenario: Migration is atomic
- **WHEN** the datamodel transform fails for A
- **THEN** no migration entry is appended and A continues under v1

#### Scenario: Migrations are in the graph
- **WHEN** an operator queries the graph for migrations of A
- **THEN** each migration is returned with its from-version, to-version, migration IRI and journal position

### Requirement: Migration Compatibility Check
Before a migration is accepted, the runtime SHALL check that the configuration mapping covers every configuration reachable in the old version, and SHALL refuse a migration that leaves any reachable configuration unmapped, listing those configurations.

#### Scenario: Unmapped configuration refused
- **WHEN** v1 can reach configuration `{Paying, Paying.Card}` and migration M maps no target for it
- **THEN** M is refused with a report naming `{Paying, Paying.Card}`

#### Scenario: Identical structure needs no mapping
- **WHEN** v2 differs from v1 only in executable content and keeps every state id
- **THEN** the runtime derives the identity mapping and the check passes without a user-supplied mapping

### Requirement: Fork Before Migration
An operator SHALL be able to fork an actor from the journal position before a migration under a new address, so that a migration can be rolled back without rewriting history.

#### Scenario: Rolling back a migration
- **WHEN** an operator forks actor A at the position before its migration to v2
- **THEN** a new actor exists with A's v1 state at that position, and A's own journal is unchanged

### Requirement: Self-Migration Requests
A chart SHALL be able to request its own migration by sending a migration request event naming a target version and migration, and the request SHALL be processed by the same checks as an operator migration, at the start of a later turn and never within the requesting macrostep.

#### Scenario: Chart requests an upgrade
- **WHEN** an actor sends a migration request for v2 with migration M during a macrostep
- **THEN** after that macrostep's entry is appended, the runtime checks M and, if accepted, appends the migration entry before processing the next input
