## Why

Every later Vertep milestone (typed authoring, actors, agents, clusters, federation) runs charts, stores charts as RDF and relies on Lean to certify them, so the statechart core must exist first and be provably right. SCXML is the only statechart standard with a published interpretation algorithm and a conformance suite, so M0 delivers the complete SCXML core, its RDF projection and its Lean model before any runtime is built.

## What Changes

- A `no_std` chart IR in `vertep-core` with well-formedness validation, document order and extension nodes.
- A pure `step` function implementing the full W3C SCXML interpretation algorithm (Appendix D): macrosteps and microsteps, optimal enabled transition sets, entry and exit order, shallow and deep history, eventless transitions, internal and external queues, done events, errors, invoke lifecycle, delayed send and cancel, emitted as deterministic effects instead of I/O.
- An SCXML 1.0 reader and writer (`vertep-scxml`) with a `vt:` extension namespace and an explicit loss report.
- The `Datamodel` trait with the `null` datamodel in the core and the `ecmascript` datamodel on boa (`vertep-dm-ecma`), with pure guards and deterministic evaluation.
- The `vt:` chart ontology module, a lossless IR to RDF 1.2 projection, SHACL shapes and RDFC-1.0 content-addressed chart IRIs (`vertep-rdf`).
- A Lean 4 package (`vertep-lean`) on LeanRDF: the IR, the executable semantics, the M0 theorems, a differential oracle and `vt certify` certificates.
- A conformance harness (`vertep-conformance`) running the W3C IRP suite on the Rust core and the Lean model, plus randomized differential testing.
- A throwaway integration spike (journal + LLM decider) run alongside M0 to surface integration risk; its code is deleted when M2 starts.

## Capabilities

### New Capabilities

- `chart-model`: the canonical chart IR, its node kinds, document order, extension nodes and well-formedness rules.
- `scxml-interpretation`: the pure step function implementing the SCXML interpretation algorithm and its effects.
- `scxml-format`: reading and writing SCXML 1.0 documents, the `vt:` extension namespace and declared loss.
- `datamodel`: the pluggable datamodel contract, the `null` and `ecmascript` datamodels, system variables and guard purity.
- `chart-rdf`: the `vt:` chart ontology, the IR to RDF 1.2 projection and back, SHACL shapes and content-addressed chart IRIs.
- `formal-model`: the Lean model of the IR and semantics, the M0 theorems, the differential oracle and chart certificates.
- `scxml-conformance`: running the W3C SCXML IRP suite and randomized differential tests against the Rust core and the Lean model.

### Modified Capabilities

None; this is the first change of the project.

## Impact

- **New crates:** `vertep-core`, `vertep-scxml`, `vertep-dm-ecma`, `vertep-rdf`, `vertep-conformance` (dev only), and the Lean package `vertep-lean`. New dependencies: `boa_engine` (feature-gated), an XML parser, an RDF 1.2 toolkit with RDFC-1.0, LeanRDF on the Lean side.
- **tg sections touched:** [[capabilities#Chart model]], [[capabilities#SCXML interpretation]], [[capabilities#SCXML format]], [[capabilities#Datamodel]], [[capabilities#Chart RDF]], [[capabilities#Formal model]], [[capabilities#SCXML conformance]], [[components#vertep-core]], [[components#vertep-scxml]], [[components#vertep-dm-ecma]], [[components#vertep-rdf]], [[components#vertep-lean]], [[components#vertep-conformance]], [[roadmap#M0 SCXML core]].
- **Decisions binding the area:** [[decisions#SCXML semantics are normative]], [[decisions#IR is the hub]], [[decisions#Pluggable datamodel]], [[decisions#Modular vt ontology on RDF 1.2]], [[decisions#Content-addressed chart versions]] (hashing only), [[decisions#Lean verification bridge]], [[decisions#No-std core and tokio shell]] (core half), [[decisions#Full SCXML first]].
- **Constraints binding the area:** [[constraints#SCXML interpretation algorithm]], [[constraints#RDF is open-world]], [[constraints#Proc macros run at compile time]], [[constraints#Aeneas supports a Rust subset]].
- **Requirements added (selection):** [[openspec:scxml-interpretation#Optimal Enabled Transition Set]], [[openspec:scxml-interpretation#Pure Step Function]], [[openspec:chart-model#Well-Formedness Validation]], [[openspec:chart-rdf#Round Trip Identity]], [[openspec:chart-rdf#Canonical Chart Hash]], [[openspec:formal-model#Shapes Equivalence Theorem]], [[openspec:formal-model#Differential Oracle]], [[openspec:scxml-conformance#Expected Deviation Allow List]]; the full list is in `specs/`.
- **Later milestones** build on this change by capability name: the type system and Rust DSL (M1), the actor runtime, event journal and chart versioning (M2), deciders (M3).
