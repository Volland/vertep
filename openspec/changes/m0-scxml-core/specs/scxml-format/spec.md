## Purpose

Reads SCXML 1.0 documents into the chart IR and writes the IR back as SCXML, so that standard charts can be imported, edited in other tools and exported, with Vertep features carried in a `vt:` extension namespace and anything unsupported reported instead of silently dropped.

## ADDED Requirements

### Requirement: SCXML Document Parsing
The SCXML reader SHALL parse documents in the SCXML namespace `http://www.w3.org/2005/07/scxml` with `version="1.0"` into the chart IR, covering every element and attribute of the SCXML 1.0 Recommendation, and SHALL return a chart that passes well-formedness validation or a list of errors.

#### Scenario: Valid document parsed
- **WHEN** a valid SCXML document with states, parallel regions, history, data and invokes is parsed
- **THEN** the reader returns a chart IR containing all of them with their attributes and document order

#### Scenario: Wrong namespace rejected
- **WHEN** the root element is `scxml` in a namespace other than the SCXML namespace
- **THEN** the reader rejects the document with an error naming the namespace

### Requirement: SCXML Serialization
The SCXML writer SHALL write any chart IR as an SCXML 1.0 document in the SCXML namespace, emitting `vt:` extension nodes in the Vertep namespace and foreign extension nodes in their own namespaces.

#### Scenario: Document written
- **WHEN** a chart IR is serialized
- **THEN** the output is a well-formed XML document with the SCXML namespace and `version="1.0"`

#### Scenario: Generated identifiers not leaked
- **WHEN** a chart contains states whose identifiers were generated
- **THEN** the writer omits those `id` attributes, unless a transition targets the state, in which case it writes the generated identifier

### Requirement: SCXML Round Trip
The reader and writer SHALL be inverses on the chart IR: parsing the serialization of a chart SHALL yield a chart equal to it, and serializing a parsed document and parsing it again SHALL yield an equal chart.

#### Scenario: IR to SCXML to IR
- **WHEN** any well-formed chart from the conformance corpus is serialized and parsed again
- **THEN** the result equals the original chart

#### Scenario: Comments and whitespace do not matter
- **WHEN** two documents differ only in comments, insignificant whitespace and attribute order
- **THEN** they parse to equal charts

### Requirement: Parse Error Reporting
The SCXML reader SHALL report malformed XML, unknown SCXML elements, missing required attributes, mutually exclusive attributes used together and well-formedness violations, each with a line and column, and SHALL report all errors it can recover from in one result.

#### Scenario: Mutually exclusive attributes
- **WHEN** a `send` declares both `event` and `eventexpr`
- **THEN** the reader reports an error at the `send` element's line and column

#### Scenario: Missing required attribute
- **WHEN** a `param` element has no `name`
- **THEN** the reader reports the missing attribute with its location

### Requirement: Source Locations
The SCXML reader SHALL attach the source location of every element to the chart node it produces so that validation errors and runtime diagnostics can point to the document, and source locations SHALL NOT take part in chart equality.

#### Scenario: Validation error points to the source
- **WHEN** a parsed chart fails validation because of an unknown transition target
- **THEN** the error carries the line and column of that transition in the source document

#### Scenario: Locations ignored by equality
- **WHEN** the same chart is parsed from two documents with different formatting
- **THEN** the charts are equal although their source locations differ

### Requirement: External Content Resolution
The SCXML reader SHALL resolve `src` attributes of `data`, `script`, `content` and `invoke` only through a resolver supplied by the caller, SHALL perform no network or file access by default, and SHALL report an unresolved source as an error.

#### Scenario: No resolver, no access
- **WHEN** a document with `<script src="lib.js"/>` is parsed without a resolver
- **THEN** the reader reports an unresolved-source error and performs no I/O

#### Scenario: Resolver supplies content
- **WHEN** the caller supplies a resolver that maps `lib.js` to a script body
- **THEN** the chart contains that script body for the element

### Requirement: Vertep Extension Namespace
The SCXML reader and writer SHALL recognize the Vertep extension namespace `https://w3id.org/vertep/vt#` and SHALL map its attributes and elements onto the chart IR's extension nodes, so that Vertep features survive a round trip through SCXML while remaining ignorable by other SCXML processors.

#### Scenario: vt attribute preserved
- **WHEN** a state carries `vt:defer="request.*"` and the document is parsed and serialized
- **THEN** the output carries the same attribute on the same state

#### Scenario: Other processors unaffected
- **WHEN** a document with `vt:` attributes is checked against the SCXML schema with foreign attributes allowed
- **THEN** the SCXML content is unchanged by the presence of the extension

### Requirement: Declared Loss Report
The SCXML reader SHALL return, with every parsed chart, a loss report listing each construct that the IR keeps only as an opaque extension or that a selected datamodel cannot execute, and SHALL fail instead in strict mode when the report is not empty.

#### Scenario: Unknown foreign element listed
- **WHEN** a document contains an element in an unknown namespace inside `onentry`
- **THEN** the chart keeps it as an opaque extension node and the loss report lists it with its location

#### Scenario: Strict mode rejects loss
- **WHEN** the same document is parsed in strict mode
- **THEN** parsing fails with the loss report as the error

### Requirement: Datamodel Attribute Handling
The SCXML reader SHALL record the root's `datamodel` attribute, defaulting to `null` when absent, and SHALL reject a document whose datamodel is not registered with the reader.

#### Scenario: Default datamodel
- **WHEN** the root declares no `datamodel`
- **THEN** the chart's datamodel is `null`

#### Scenario: Unregistered datamodel
- **WHEN** the root declares `datamodel="xpath"` and no xpath datamodel is registered
- **THEN** the reader rejects the document with an unsupported-datamodel error
