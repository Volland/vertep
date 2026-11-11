## Purpose

Represents every chart as RDF 1.2 under the `vt:` chart ontology, losslessly and validated by SHACL shapes, and names each chart by the hash of its canonical RDF, so charts can be stored, queried, linked and versioned in any RDF store while the IR stays the hub.

## ADDED Requirements

### Requirement: Chart Ontology Module
The project SHALL publish the `vt:` chart ontology module in Turtle under the namespace `https://w3id.org/vertep/vt#`, with one class per IR node kind (chart, state kinds, transition, executable content elements, data item, invoke, extension node) and one property per IR attribute and containment relation, each with a label and a definition.

#### Scenario: Every IR node kind has a class
- **WHEN** the ontology module is compared with the list of IR node kinds
- **THEN** every node kind has exactly one class and every class corresponds to a node kind

#### Scenario: Ontology parses
- **WHEN** the ontology Turtle file is parsed by a conforming RDF 1.2 parser
- **THEN** it parses without errors

### Requirement: IR To RDF Projection
The RDF projection SHALL map every chart to an RDF 1.2 graph in which every IR node, attribute, expression text, extension node and containment relation is represented, with node IRIs derived deterministically from the chart IRI and the node's identity.

#### Scenario: State and transition triples
- **WHEN** a chart with state `a` and a transition from `a` to `b` on event `go` is projected
- **THEN** the graph types `a` as a `vt:` state class, links the transition to `a` as source and `b` as target, and records `go` as its event descriptor

#### Scenario: Deterministic node IRIs
- **WHEN** the same chart is projected twice
- **THEN** both graphs are identical, including node IRIs

### Requirement: RDF To IR Reconstruction
The RDF projection SHALL reconstruct a chart from a graph that conforms to the chart shapes, and SHALL reject a graph that does not conform with the SHACL validation report as the error.

#### Scenario: Conforming graph reconstructed
- **WHEN** a graph produced by the projection is given to reconstruction
- **THEN** reconstruction returns a chart

#### Scenario: Missing required triple
- **WHEN** a graph lacks the source of a transition
- **THEN** reconstruction fails with a validation report naming the transition and the missing property

### Requirement: Round Trip Identity
Reconstruction after projection SHALL be the identity on charts: for every well-formed chart, reconstructing the projected graph SHALL yield a chart equal to the original, and this SHALL hold for every chart of the SCXML conformance corpus.

#### Scenario: Conformance corpus round trip
- **WHEN** every chart parsed from the conformance corpus is projected and reconstructed
- **THEN** each result equals its original chart

#### Scenario: Extensions survive the round trip
- **WHEN** a chart with `vt:` and foreign extension nodes is projected and reconstructed
- **THEN** the extension nodes are equal, in the same positions

### Requirement: Explicit Ordering
The RDF projection SHALL record document order and the order of every ordered collection with an explicit integer `vt:order` on each member, and SHALL NOT rely on triple order or on RDF collections for document order.

#### Scenario: Transition priority recoverable
- **WHEN** a state's transitions are projected and the triples are shuffled
- **THEN** reconstruction returns the transitions in their original document order

#### Scenario: Target order recoverable
- **WHEN** a transition with targets `p.b p.a` is projected
- **THEN** the targets carry `vt:order` values that reconstruct the order `p.b`, `p.a`

### Requirement: Executable Content As Literals
The RDF projection SHALL store every expression, condition, location and script text as a literal whose datatype names the chart's datamodel, and SHALL store inline content as a literal that preserves its exact text.

#### Scenario: Condition literal typed by datamodel
- **WHEN** an ecmascript chart's transition has `cond="x > 1"`
- **THEN** the projected condition is the literal `x > 1` with the Vertep ecmascript expression datatype

#### Scenario: Inline content preserved exactly
- **WHEN** a data item has inline XML content
- **THEN** the projected literal reproduces the content exactly after reconstruction

### Requirement: SHACL Shapes For Charts
The project SHALL publish a SHACL shapes graph for the chart module such that a graph conforms to the shapes exactly when it is the projection of a well-formed chart, covering every well-formedness rule of the chart model that can be expressed over the graph.

#### Scenario: Well-formed chart conforms
- **WHEN** the projection of a well-formed chart is validated against the shapes
- **THEN** the validation report conforms

#### Scenario: Ill-formed graph rejected
- **WHEN** a graph has a transition targeting a node that is not a state of the chart
- **THEN** the validation report does not conform and names the transition

#### Scenario: Shapes and IR validation agree
- **WHEN** a corpus of generated charts, both well-formed and ill-formed, is checked by IR validation and by the shapes on their projections
- **THEN** the two verdicts are equal for every chart

### Requirement: Canonical Chart Hash
The RDF projection SHALL compute a chart's hash as the SHA-256 of the RDFC-1.0 canonical N-Quads of its projection, so that the hash depends only on the chart and not on the surface it was written in or on serialization details.

#### Scenario: Same chart from two surfaces
- **WHEN** the same chart is written once in SCXML with one formatting and once with another formatting and attribute order
- **THEN** both charts have the same hash

#### Scenario: Any semantic change changes the hash
- **WHEN** one transition's condition text is changed
- **THEN** the hash changes

### Requirement: Chart Version IRI
The RDF projection SHALL name each chart version by the IRI `urn:vertep:chart:sha256:<hex>` formed from its canonical hash, and SHALL use that IRI as the base of the chart's node IRIs.

#### Scenario: Version IRI from hash
- **WHEN** a chart's canonical hash is computed
- **THEN** its version IRI is `urn:vertep:chart:sha256:` followed by the lowercase hex digest

#### Scenario: Node IRIs scoped by version
- **WHEN** two different versions of a chart both contain state `a`
- **THEN** the two state nodes have different IRIs

### Requirement: Annotations Outside The Projection
Reconstruction and hashing SHALL ignore triples that are not part of the chart projection, including RDF 1.2 triple-term annotations about chart nodes, so that charts can be annotated in a store without changing their identity.

#### Scenario: Annotation does not change the chart
- **WHEN** a graph contains a chart projection plus an annotation reifying one of its transition triples with a comment
- **THEN** reconstruction returns the original chart and the hash is unchanged

#### Scenario: Annotation does not break validation
- **WHEN** the annotated graph is validated against the chart shapes
- **THEN** the report conforms

### Requirement: RDF Serialization Formats
The RDF projection SHALL read and write chart graphs as Turtle, N-Triples, N-Quads and TriG in their RDF 1.2 versions, placing a chart in a named graph whose name is its version IRI when a dataset format is used.

#### Scenario: TriG named graph
- **WHEN** a chart is written as TriG
- **THEN** its triples are in the named graph whose name is the chart's version IRI

#### Scenario: Format independence
- **WHEN** the same chart is written as Turtle and as N-Quads and both are read back
- **THEN** both reconstruct to equal charts
