## ADDED Requirements

### Requirement: Benchmark Suite
The repository SHALL contain a benchmark suite for pure step throughput, journal append latency and replay speed per storage backend, scripted-decider round trip, and the size and cold start of `@vertep/web` and the wheel. A release MUST record its results, and CI MUST fail a change that regresses a measured figure beyond a stated threshold against the last release.

#### Scenario: Results are recorded
- **WHEN** a release is cut
- **THEN** the results page lists each figure with the machine and the commit

#### Scenario: A regression fails CI
- **WHEN** a change makes journal replay slower than the threshold against the last recorded result
- **THEN** CI fails and names the figure
