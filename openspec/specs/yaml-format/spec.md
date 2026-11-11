# yaml-format Specification

## Purpose
Provide a concise, human-editable YAML surface for Vertep charts that maps one to one onto the chart IR, including types, so YAML charts convert losslessly to SCXML, RDF and the typed Rust API.

## Requirements

### Requirement: YAML Chart Document
A YAML chart SHALL be a single YAML 1.2 document with a top-level mapping containing a required `vertep` format version, a required `name`, an optional `datamodel` (default `cel`), an optional `initial`, an optional `types` block, an optional `context` block and a required `states` mapping.

#### Scenario: Minimal chart
- **WHEN** a document declares `vertep: 1`, `name: light` and `states` with two atomic states
- **THEN** it loads into an IR chart named `light` with the CEL datamodel and the first declared state as initial

#### Scenario: Missing format version
- **WHEN** a document has no `vertep` key
- **THEN** loading fails with an error stating that the format version is required

#### Scenario: Unsupported format version
- **WHEN** `vertep: 99` is declared
- **THEN** loading fails with an error naming the supported versions

### Requirement: States and Hierarchy
The `states` mapping SHALL key each state by its id and SHALL express compound states by a nested `states` mapping, parallel states by `type: parallel`, final states by `type: final`, and history states by `type: history` with `history: shallow` or `deep`, preserving document order of states as declared.

#### Scenario: Compound state with initial child
- **WHEN** state `checkout` declares nested states `cart` and `payment` and `initial: cart`
- **THEN** the IR has compound state `checkout` with initial child `cart`

#### Scenario: Document order preserved
- **WHEN** a parallel state declares regions `a`, `b`, `c` in that order
- **THEN** the IR records them in the same order, which governs entry and exit order

#### Scenario: Duplicate state id
- **WHEN** two states anywhere in the document have id `done`
- **THEN** loading fails with errors pointing at both lines

### Requirement: Transitions
Each state SHALL declare transitions under `on`, keyed by event descriptor, and under `always` for eventless transitions, where a transition is a target string, a list of targets, or a mapping with `target`, `cond`, `type` (`external` or `internal`) and `actions`, and several transitions for one descriptor SHALL be a list evaluated in document order.

#### Scenario: Shorthand target
- **WHEN** state `idle` declares `on: { start: running }`
- **THEN** the IR has a transition from `idle` on `start` to `running`

#### Scenario: Guarded alternatives in order
- **WHEN** `on: { pay: [ { cond: "ev.amount > 100", target: review }, { target: paid } ] }` is declared
- **THEN** the IR has two transitions on `pay` in that order, the first guarded

#### Scenario: Eventless transition
- **WHEN** a state declares `always: [ { cond: "ctx.ready", target: next } ]`
- **THEN** the IR has an eventless guarded transition to `next`

### Requirement: Executable Content
Entry actions, exit actions and transition actions SHALL be lists of executable content items, each a mapping with exactly one key among `assign`, `raise`, `send`, `log`, `if`, `foreach`, `cancel`, `script` and `call`, mapping one to one to SCXML executable content and to named function calls of the datamodel.

#### Scenario: Assign and send
- **WHEN** an entry list contains `{ assign: { location: ctx.total, expr: "ctx.total + ev.amount" } }` and `{ send: { event: receipt, target: "#_parent" } }`
- **THEN** the IR contains an `<assign>` followed by a `<send>` with the same attributes

#### Scenario: Two keys in one item
- **WHEN** one item contains both `raise` and `log`
- **THEN** loading fails with an error at that item stating that exactly one key is allowed

### Requirement: Invoke and Data in YAML
States SHALL declare invocations under `invoke` with `type`, `src` or inline content, `id`, `params`, `autoforward` and `finalize`, and context data SHALL be declared under `context` with a type and an initial expression per field.

#### Scenario: Invoke with params
- **WHEN** state `fetching` declares `invoke: { id: loader, type: "vt:actor", src: "vt:actor/Loader", params: { url: "ctx.url" } }`
- **THEN** the IR contains an `<invoke>` with that id, type, source and a param `url`

#### Scenario: Typed context field
- **WHEN** `context: { total: { type: decimal, init: "0" } }` is declared
- **THEN** the context type has field `total` of type `decimal` initialized to `0`

### Requirement: YAML Types Block
The `types` block SHALL declare named types with the type language's full expressiveness using records as mappings of field to type, unions under `oneOf` with tags, `list<T>`, `optional<T>` and primitive names, and events SHALL declare payload types under a top-level `events` mapping.

#### Scenario: Record and union types
- **WHEN** `types` declares `Order: { id: string, total: decimal }` and `Payment: { oneOf: { Card: { last4: string }, Cash: null } }`
- **THEN** the chart has named types `Order` and `Payment` with the declared structure

#### Scenario: Event payload type
- **WHEN** `events: { pay: Payment }` is declared
- **THEN** event `pay` has payload type `Payment` and is checked by the type checker

### Requirement: Lossless YAML Round Trip
Converting a YAML chart to the IR and back to YAML SHALL yield a document that loads to the same IR, and converting any IR chart, including one read from SCXML, to YAML and back SHALL yield the same IR and chart version IRI.

#### Scenario: SCXML to YAML to IR
- **WHEN** every chart of the SCXML conformance suite is converted to YAML and loaded again
- **THEN** each loaded IR equals the IR read from the original SCXML

#### Scenario: Stable output
- **WHEN** an IR chart is written to YAML twice
- **THEN** both outputs are byte-identical

### Requirement: Source-Located Errors
Every YAML loading, validation and type error SHALL report the file path, line and column of the offending node and a dotted path to it (for example `states.checkout.on.pay[1].target`), and all errors in a document SHALL be reported together.

#### Scenario: Unknown target located
- **WHEN** a transition at line 14 targets `paymnt`
- **THEN** the error reports line 14, the path to the transition and the unknown target `paymnt`

#### Scenario: YAML syntax error
- **WHEN** the document has an indentation error
- **THEN** loading fails with the parser's line and column and no IR is produced

### Requirement: YAML Uses Expression Strings of Its Datamodel
Expressions in a YAML chart (conditions, assignment expressions, send payloads, params) SHALL be strings in the language of the chart's declared datamodel, CEL by default, and SHALL be parsed and type-checked when the chart loads.

#### Scenario: CEL expression parse error
- **WHEN** a `cond` contains `ctx.total >`
- **THEN** loading fails with a parse error located inside that string

#### Scenario: Different datamodel declared
- **WHEN** a YAML chart declares `datamodel: ecmascript`
- **THEN** its expression strings are parsed as ECMAScript instead of CEL

### Requirement: Command Line Conversion
The `vt convert <input> --to scxml|yaml|turtle|trig|nquads` command SHALL convert a chart between every supported surface, detecting the input surface from its extension or content, and SHALL refuse to write output when the conversion would lose information, listing what would be lost.

#### Scenario: YAML to SCXML
- **WHEN** `vt convert order.yaml --to scxml` is run
- **THEN** it writes an SCXML document whose IR equals the YAML chart's IR, with types as `vt:type` attributes

#### Scenario: Conversion to RDF
- **WHEN** `vt convert order.scxml --to turtle` is run
- **THEN** it writes the chart's RDF 1.2 projection in Turtle

#### Scenario: Lossy conversion refused
- **WHEN** a conversion would drop a construct the target surface cannot express
- **THEN** the command exits non-zero, lists each construct that would be lost and writes no output
