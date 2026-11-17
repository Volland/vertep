## MODIFIED Requirements

### Requirement: Native Library Is Optional
The native engine library SHALL sit behind a cargo feature that is off by default. Programs inside the portable profile MUST run without it on the portable engine. On a build without it, loading a chart whose rules decider runs a program outside the portable profile MUST fail with a typed `rules-unavailable` error naming the first non-portable clause, and MUST NOT fall back to another decider.

#### Scenario: Browser build
- **WHEN** a chart with a rules decider and a program outside the portable profile is loaded in the WebAssembly build
- **THEN** loading fails with `rules-unavailable` naming the clause

#### Scenario: Browser build with a portable program
- **WHEN** a chart with a rules decider and a portable program is loaded in the WebAssembly build
- **THEN** it loads and decides on the portable engine

#### Scenario: Feature off
- **WHEN** the workspace is built with default features
- **THEN** it builds and links no native engine library
