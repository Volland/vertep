## ADDED Requirements

### Requirement: Layout Namespace
The SCXML reader SHALL treat every attribute and element in the layout namespace `https://w3id.org/vertep/layout#` (conventional prefix `vl`) as formatting: it SHALL drop them from the chart IR, list each in the formatting part of the loss report, never fail strict mode because of them, and never turn them or the namespace declaration into extension nodes. The writer SHALL never emit them.

#### Scenario: Layout dropped from the IR
- **WHEN** a document carries `vl:x`, `vl:y`, `vl:w`, `vl:h` and `vl:collapsed` on its states and `vl:route` and `vl:label` on its transitions, with `xmlns:vl` declared
- **THEN** it reads to the same chart as the document without them, and the loss report lists each as a formatting layout loss

#### Scenario: Strict mode accepts layout
- **WHEN** the same document is read in strict mode
- **THEN** reading succeeds

#### Scenario: Layout does not change the chart version
- **WHEN** the chart hash of the document with layout and of the document without it are computed
- **THEN** both give the same hash and version IRI

#### Scenario: Other foreign namespaces unchanged
- **WHEN** a state carries both a `vl:` attribute and an attribute in another foreign namespace
- **THEN** only the other attribute is kept as an extension node and reported as a foreign extension, and strict mode still fails on it
