## Purpose

Give every Vertep chart one algebraic type language for event payloads, context and executable content, checked before a chart runs and projected consistently to Rust types, JSON Schema, SHACL shapes and a CEL type environment.

## ADDED Requirements

### Requirement: Primitive Types
The type language SHALL provide exactly the primitive types `bool`, `int`, `decimal`, `string`, `datetime` and `iri`, where `int` is a signed 64-bit integer, `decimal` is an arbitrary-precision decimal, `datetime` is an RFC 3339 instant with an explicit offset and `iri` is an absolute IRI per RFC 3987.

#### Scenario: Integer outside 64-bit range is rejected
- **WHEN** a value `9223372036854775808` is checked against `int`
- **THEN** the check fails with a type error naming the expected type `int` and the value's location

#### Scenario: Datetime without offset is rejected
- **WHEN** the value `2026-11-11T10:00:00` without an offset is checked against `datetime`
- **THEN** the check fails with a type error stating that an offset is required

#### Scenario: Relative IRI is rejected
- **WHEN** the value `orders/42` is checked against `iri`
- **THEN** the check fails because the IRI is not absolute

### Requirement: Composite Types
The type language SHALL provide records with named fields, tagged unions whose variants each carry an optional payload type, homogeneous lists, and optional values, and these constructors SHALL nest to any depth.

#### Scenario: Record with a missing required field
- **WHEN** a value lacking field `amount` is checked against the record `{ id: string, amount: decimal }`
- **THEN** the check fails with an error naming the missing field `amount`

#### Scenario: Record with an unknown field
- **WHEN** a value with an extra field `note` is checked against a record that does not declare `note`
- **THEN** the check fails with an error naming the unknown field `note`

#### Scenario: Tagged union selects its variant by tag
- **GIVEN** the union `Payment = Card { last4: string } | Cash`
- **WHEN** a value with tag `Cash` and no payload is checked
- **THEN** the check succeeds
- **AND** a value with tag `Card` and no payload fails with an error naming the variant `Card`

#### Scenario: Optional accepts absence
- **WHEN** a record field of type `optional<string>` is absent from a value
- **THEN** the check succeeds and the field reads as absent, not as an empty string

### Requirement: Named Content-Addressed Types
A named type SHALL be identified by an IRI derived from the hash of its canonical definition, so two structurally identical definitions under the same name receive the same type IRI and any change to a definition produces a different type IRI.

#### Scenario: Identical definitions share an IRI
- **WHEN** the type `Order { id: string, total: decimal }` is declared in two different charts
- **THEN** both declarations resolve to the same type IRI

#### Scenario: Changed definition gets a new IRI
- **WHEN** a field `currency: string` is added to `Order`
- **THEN** the new definition has a type IRI different from the previous one
- **AND** charts referencing the previous IRI keep referring to the previous definition

#### Scenario: Field order does not change identity
- **WHEN** two record definitions list the same fields with the same types in different source order
- **THEN** they resolve to the same type IRI

### Requirement: Recursive and Cyclic Type Definitions
Named types SHALL be allowed to refer to themselves or to each other only through a list or an optional, and a definition that is infinitely sized without such indirection SHALL be rejected.

#### Scenario: Recursive tree through a list
- **WHEN** the type `Node { label: string, children: list<Node> }` is declared
- **THEN** the declaration is accepted

#### Scenario: Unguarded self reference
- **WHEN** the type `Loop { next: Loop }` is declared
- **THEN** the declaration is rejected with an error identifying the cycle `Loop -> Loop`

### Requirement: Opt-In Chart Typing
A chart SHALL be typed only when it declares types for its context or events; an untyped chart SHALL run with every event payload and context value of the dynamic type `value`, and SHALL execute exactly as in M0.

#### Scenario: Untyped SCXML chart runs unchanged
- **WHEN** an SCXML chart with no `vt:type` attributes is loaded
- **THEN** it passes loading without type errors
- **AND** its execution trace is identical to its M0 trace on the same inputs

#### Scenario: Partially typed chart
- **WHEN** a chart declares a type for event `pay` and no type for event `cancel`
- **THEN** payloads of `pay` are checked against the declared type
- **AND** payloads of `cancel` are accepted as `value`

### Requirement: Dynamic Value Boundary
The dynamic type `value` SHALL accept any JSON-compatible or RDF-literal value, and a `value` SHALL be usable where a static type is expected only through an explicit checked conversion that raises a type error on mismatch.

#### Scenario: Assigning dynamic data to typed context
- **WHEN** an action assigns an untyped event payload to a context field of type `int` through a checked conversion and the payload is the string `"7"`
- **THEN** the conversion fails and the runtime raises `error.execution` with a type mismatch description

#### Scenario: Static to dynamic is always allowed
- **WHEN** a typed context value is passed where `value` is expected
- **THEN** no check is performed and the value is accepted

### Requirement: Chart Type Checking
Before a typed chart can be instantiated, the system SHALL check that every typed event payload, context declaration, `<data>` initializer, `<send>` payload, `<param>`, guard and action is consistent with the declared types, and SHALL report every error found rather than stopping at the first.

#### Scenario: Send with a payload of the wrong shape
- **GIVEN** event `pay` is declared with payload `{ amount: decimal }`
- **WHEN** a `<send event="pay">` supplies a field `amount` of type `string`
- **THEN** type checking fails with an error locating the `<send>` and naming field `amount`

#### Scenario: Guard reads an undeclared context field
- **WHEN** a guard references context field `limit` that the context type does not declare
- **THEN** type checking fails with an error naming `limit` and the guard's transition

#### Scenario: Several errors are reported together
- **WHEN** a chart contains three independent type errors
- **THEN** type checking reports all three, each with its location

#### Scenario: Instantiation is refused after type errors
- **WHEN** a chart with type errors is instantiated
- **THEN** instantiation fails and no session is created

### Requirement: Event Type Declarations
Each typed event name SHALL have at most one payload type, an event descriptor that matches several typed events through a prefix or wildcard SHALL see the payload as the union of their payload types, and system events defined by SCXML (`done.state.*`, `done.invoke.*`, `error.*`) SHALL have predefined types.

#### Scenario: Conflicting declarations for one event
- **WHEN** event `pay` is declared with two different payload types
- **THEN** type checking fails with an error naming `pay` and both declarations

#### Scenario: Prefix descriptor sees a union
- **GIVEN** events `order.created` and `order.cancelled` with different payload types
- **WHEN** a transition with descriptor `order` reads the payload
- **THEN** the guard sees a union of the two payload types and must discriminate before reading variant-specific fields

#### Scenario: Error events are typed
- **WHEN** a transition on `error.execution` reads `_event.data`
- **THEN** the payload has the predefined error type with fields for the error kind and message

### Requirement: Rust Projection
Every IR type SHALL project to a Rust type in which records become structs, tagged unions become enums, lists become `Vec`, optional values become `Option`, primitives become fixed documented Rust types, and every generated type SHALL serialize to and from the canonical value encoding without loss.

#### Scenario: Union projects to an enum
- **WHEN** the type `Payment = Card { last4: string } | Cash` is projected to Rust
- **THEN** the result is an enum with variants `Card { last4: String }` and `Cash`

#### Scenario: Round trip through Rust
- **WHEN** any well-typed value is converted to its Rust projection and back
- **THEN** the result equals the original value

### Requirement: JSON Schema Projection
Every IR type SHALL project to a JSON Schema (draft 2020-12) such that a JSON document validates against the schema exactly when its decoding is a value of the IR type, and the projection SHALL be accepted by structured-output interfaces that support JSON Schema object, enum and `anyOf` keywords.

#### Scenario: Schema agrees with the checker
- **WHEN** a JSON document is validated against the projected schema and also checked against the IR type
- **THEN** both checks give the same verdict, for every document in a generated test corpus

#### Scenario: Tagged union in JSON Schema
- **WHEN** a tagged union is projected
- **THEN** the schema uses a discriminating tag property with a `const` per variant inside `anyOf`

#### Scenario: Closed records
- **WHEN** a record type is projected
- **THEN** the schema sets `additionalProperties` to false and lists the required fields

### Requirement: SHACL Projection
Every IR type SHALL project to SHACL Core shapes such that the RDF encoding of a value conforms to the shapes exactly when the value is of the IR type, and the shapes SHALL be published in the chart's RDF under the type's IRI.

#### Scenario: SHACL agrees with the checker
- **WHEN** the RDF encoding of a value is validated against the projected shapes and the value is also checked against the IR type
- **THEN** both give the same verdict, for every value in a generated test corpus

#### Scenario: Shapes are found by type IRI
- **WHEN** a chart is projected to RDF
- **THEN** each named type it declares appears as a node shape identified by the type IRI

### Requirement: CEL Type Environment Projection
The declared context, event payload and named types of a chart SHALL project to a CEL type environment in which records are message types, unions are discriminated message types, lists are `list`, optional values are optional types and primitives map to documented CEL types, so CEL expressions can be type-checked against the chart.

#### Scenario: Field access is checked
- **WHEN** a CEL guard `ctx.total > 100` is checked and `total` is a `decimal` field of the context
- **THEN** the expression type-checks as `bool`

#### Scenario: Unknown field fails checking
- **WHEN** a CEL guard reads `ctx.missing`
- **THEN** type checking fails with an error naming `missing`

### Requirement: Typed SCXML Attributes
SCXML documents SHALL carry types through the extension attribute `vt:type` on `<data>`, `<send>` and `<param>` and through a `vt:types` element declaring named types, and a document with these extensions SHALL still be valid SCXML that a processor ignoring the `vt:` namespace can run untyped.

#### Scenario: Typed data element
- **WHEN** `<data id="total" vt:type="decimal" expr="0"/>` is loaded
- **THEN** the context field `total` has type `decimal`

#### Scenario: Foreign processor ignores types
- **WHEN** a typed Vertep SCXML document is given to an SCXML processor that does not know the `vt:` namespace
- **THEN** the document is still well-formed SCXML and the extension attributes are the only difference from an untyped chart

#### Scenario: Unknown type name
- **WHEN** `vt:type="Ordr"` names a type that is not declared
- **THEN** loading fails with an error naming `Ordr` and suggesting the closest declared name

### Requirement: Types Survive Every Surface
The types of a chart SHALL be preserved without loss when the chart is converted between SCXML, YAML, the Rust DSL's IR and RDF, so converting a typed chart through any sequence of surfaces and back yields the same types and the same chart version IRI.

#### Scenario: YAML to SCXML to RDF and back
- **WHEN** a typed YAML chart is converted to SCXML, then to RDF, then back to YAML
- **THEN** the resulting chart has the same types and the same chart version IRI as the original

#### Scenario: Type change changes the chart version
- **WHEN** only the payload type of one event changes
- **THEN** the chart version IRI changes

### Requirement: Command Line Type Tools
The `vt` command SHALL provide `vt typecheck <chart>` to type-check a chart in any surface and report every error with its location, `vt schema <chart> --format rust|jsonschema|shacl|cel` to print a projection of the chart's types, `vt validate <chart>` to check well-formedness and SHACL conformance, `vt hash <chart>` to print the chart version IRI, and `vt certify <chart>` to run the Lean checker and print the resulting certificate, each exiting non-zero on failure.

#### Scenario: Typecheck failure exit code
- **WHEN** `vt typecheck order.yaml` is run on a chart with two type errors
- **THEN** both errors are printed with file, line and column and the command exits with a non-zero status

#### Scenario: Schema export
- **WHEN** `vt schema order.scxml --format jsonschema` is run
- **THEN** it prints a JSON Schema document per event payload type and for the context type

#### Scenario: Hash is surface independent
- **WHEN** `vt hash` is run on the same chart written in SCXML and in YAML
- **THEN** both runs print the same chart version IRI

#### Scenario: Certify without the Lean checker
- **WHEN** `vt certify` is run and the Lean checker binary is not installed
- **THEN** the command exits non-zero with a message explaining how to install the checker and no certificate is printed
