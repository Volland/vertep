## MODIFIED Requirements

### Requirement: YAML Chart Document
A YAML chart SHALL be a single YAML 1.2 document with a top-level mapping containing a required `vertep` format version, a required `name`, an optional `datamodel` (default `cel`), an optional `initial`, an optional `types` block, an optional `context` block, a required `states` mapping and an optional `layout` block.

#### Scenario: Minimal chart
- **WHEN** a document declares `vertep: 1`, `name: light` and `states` with two atomic states
- **THEN** it loads into an IR chart named `light` with the CEL datamodel and the first declared state as initial

#### Scenario: Missing format version
- **WHEN** a document has no `vertep` key
- **THEN** loading fails with an error stating that the format version is required

#### Scenario: Unsupported format version
- **WHEN** `vertep: 99` is declared
- **THEN** loading fails with an error naming the supported versions

## ADDED Requirements

### Requirement: Layout Block
The YAML reader SHALL accept an optional top-level `layout` key with any YAML value as a designer's visual layout and SHALL ignore it, so that it never reaches the chart IR or the chart hash and never causes a schema error, and the YAML writer SHALL never emit it.

#### Scenario: Layout ignored by the reader
- **WHEN** a document adds `layout: { states: { green: { x: 40, y: 40, w: 120, h: 60 } }, transitions: { "green#0": { route: [[160, 70], [200, 70]], label: [180, 60] } } }` to a chart
- **THEN** it loads into the same chart as the document without it

#### Scenario: Layout never written
- **WHEN** a chart loaded from a document with a `layout` block is written as YAML
- **THEN** the output has no `layout` key and equals the output for the document without it

#### Scenario: Layout does not change the chart version
- **WHEN** the chart hash of the document with layout and of the document without it are computed
- **THEN** both give the same hash and version IRI
