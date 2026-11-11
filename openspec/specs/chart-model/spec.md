# chart-model Specification

## Purpose
Defines the canonical chart IR that every Vertep surface (SCXML, YAML, the Rust macros, RDF) lowers into and that the interpreter executes: its node kinds, document order, extension nodes and the well-formedness rules a chart must satisfy before it can run.

## Requirements

### Requirement: Chart Document Structure
The chart model SHALL represent a chart as a root element with an optional name, a datamodel name, a binding mode (early or late), an initial target set, a top-level datamodel block, an optional top-level script, and an ordered list of child states, covering every element of the SCXML 1.0 core, executable content, datamodel and external-communication modules (W3C SCXML sections 3, 4, 5 and 6).

#### Scenario: Root with nested states
- **WHEN** a chart is built with a root containing a compound state `a` that contains atomic states `a1` and `a2`, and a final state `done`
- **THEN** the chart model holds a root with two children in that order, `a` classified as compound and `a1`, `a2` and `done` classified by their kind

#### Scenario: Every SCXML element kind is representable
- **WHEN** a chart uses `state`, `parallel`, `final`, `history`, `initial`, `transition`, `onentry`, `onexit`, `datamodel`, `data`, `donedata`, `content`, `param`, `invoke`, `finalize` and every executable content element
- **THEN** each of them has a distinct representation in the chart model with all of its SCXML attributes

### Requirement: State Kinds
The chart model SHALL classify every state as exactly one of atomic, compound, parallel, final or history, where a `state` with child states is compound, a `state` without child states is atomic, and history states carry a shallow or deep type.

#### Scenario: Compound versus atomic
- **WHEN** a `state` element has at least one child `state`, `parallel` or `final`
- **THEN** it is classified as compound, and a `state` with none of those children is classified as atomic

#### Scenario: History type defaults to shallow
- **WHEN** a history state declares no type
- **THEN** the chart model records it as a shallow history state

### Requirement: State Identity
The chart model SHALL give every state a unique identifier; states declared without an `id` SHALL receive a generated identifier that is deterministic for a given chart and cannot collide with any declared identifier.

#### Scenario: Generated identifiers are stable
- **WHEN** the same chart containing anonymous states is lowered twice
- **THEN** the generated identifiers are identical in both lowerings

#### Scenario: Generated identifiers avoid declared names
- **WHEN** a chart declares a state whose `id` equals the form a generated identifier would take
- **THEN** the generated identifier for the anonymous state differs from every declared identifier

### Requirement: Document Order
The chart model SHALL record a total document order over all states, transitions and executable content elements, and every ordered collection (children, transitions of a state, targets of a transition, executable content blocks) SHALL preserve the order of the source.

#### Scenario: Transition order preserved
- **WHEN** a state declares transitions `t1`, `t2` and `t3` in that order
- **THEN** the chart model returns them in the order `t1`, `t2`, `t3` and each carries a document position smaller than the next

#### Scenario: Multiple targets keep their order
- **WHEN** a transition declares `target="p.b p.a"`
- **THEN** the chart model keeps the targets in the order `p.b`, `p.a`

### Requirement: Transition Structure
The chart model SHALL represent a transition with its source state, an ordered and possibly empty target list, a possibly empty list of event descriptors, an optional condition, a type of internal or external (external by default), and an executable content block.

#### Scenario: Targetless transition
- **WHEN** a transition declares an event and executable content but no target
- **THEN** the chart model records an empty target list and the transition is targetless

#### Scenario: Eventless transition
- **WHEN** a transition declares no `event` attribute
- **THEN** the chart model records an empty event descriptor list and the transition is eventless

### Requirement: Initial State Resolution
The chart model SHALL resolve the initial child of every compound state and of the root from the `initial` attribute, else from an `<initial>` element's transition, else the first child state in document order, and SHALL record the resolved default entry for each.

#### Scenario: Initial attribute wins
- **WHEN** a compound state has `initial="s2"` and children `s1`, `s2`
- **THEN** the resolved default entry of the compound state is `s2`

#### Scenario: First child by default
- **WHEN** a compound state has neither an `initial` attribute nor an `<initial>` element
- **THEN** the resolved default entry is its first child state in document order

### Requirement: Executable Content Model
The chart model SHALL represent the executable content elements `raise`, `send`, `cancel`, `log`, `assign`, `script`, `if`/`elseif`/`else` and `foreach` with all of their attributes, including the `expr` variants (`eventexpr`, `targetexpr`, `typeexpr`, `idlocation`, `delayexpr`) and `namelist`, as uninterpreted expression text tagged with the chart's datamodel.

#### Scenario: Send with expressions
- **WHEN** a `send` declares `eventexpr`, `targetexpr`, `delay` and a `namelist`
- **THEN** the chart model keeps each attribute with its expression text and the datamodel name, without evaluating it

#### Scenario: Conditional branches
- **WHEN** an `if` has one `elseif` and one `else`
- **THEN** the chart model holds three ordered branches, the first two with conditions and the last without

### Requirement: Invoke Declaration
The chart model SHALL represent an `invoke` with its type or type expression, source or source expression, identifier or identifier location, `namelist`, parameters, inline content, `autoforward` flag and `finalize` block.

#### Scenario: Invoke with inline content
- **WHEN** an `invoke` of type `scxml` declares an inline `<content>` chart and `autoforward="true"`
- **THEN** the chart model holds the inline content, the type and the autoforward flag set

#### Scenario: Generated invoke identifier location
- **WHEN** an `invoke` declares `idlocation` instead of `id`
- **THEN** the chart model records the location so the interpreter can store the generated identifier there

### Requirement: Datamodel Declaration
The chart model SHALL represent every `data` item with its identifier, its `src`, `expr` or inline content, and the state that declares it, and SHALL record the chart's binding mode.

#### Scenario: Data declared in a nested state
- **WHEN** state `s` declares `<data id="x" expr="1"/>`
- **THEN** the chart model records data item `x` as owned by `s`

#### Scenario: Late binding recorded
- **WHEN** the root declares `binding="late"`
- **THEN** the chart model records late binding for the chart

### Requirement: Extension Nodes
The chart model SHALL keep every attribute and element from the `vt:` namespace or any foreign namespace as an extension node attached to its owning node, keyed by namespace IRI and local name, preserving its content and its document position.

#### Scenario: Vertep extension attribute
- **WHEN** a state carries an attribute in the `vt:` namespace
- **THEN** the chart model attaches it to that state as an extension keyed by the Vertep namespace IRI and the attribute name

#### Scenario: Foreign element preserved
- **WHEN** a transition contains an element in an unknown namespace
- **THEN** the chart model keeps it as an opaque extension node in its document position

### Requirement: Well-Formedness Validation
The chart model SHALL validate a chart and accept it only when it is well-formed, and SHALL otherwise return every violation found with the offending node, never only the first; the rules SHALL include unique state identifiers, existing transition targets, legal targets for `initial` attributes and `<initial>` transitions (descendants of the owning state), history states with at most one transition that has no event and no condition and targets descendants of the history's parent, final states without children other than `onentry`, `onexit` and `donedata`, no transitions out of final states, and parallel states whose children are not history-only.

#### Scenario: Unknown target rejected
- **WHEN** a transition targets a state identifier that does not exist
- **THEN** validation fails with an error naming the transition and the unknown identifier

#### Scenario: Several violations reported together
- **WHEN** a chart has a duplicate state identifier and an illegal `initial` attribute
- **THEN** validation returns both violations in one result

#### Scenario: Valid chart accepted
- **WHEN** a chart satisfies every rule
- **THEN** validation succeeds and the chart can be executed

### Requirement: Chart Equality
The chart model SHALL define structural equality over charts that compares every node, attribute, expression text, extension node and document order, so that two charts are equal exactly when every surface would serialize them identically.

#### Scenario: Order matters for equality
- **WHEN** two charts differ only in the order of two transitions of one state
- **THEN** they are not equal

#### Scenario: Equal after lowering the same source twice
- **WHEN** the same source document is lowered twice
- **THEN** the two charts are equal
