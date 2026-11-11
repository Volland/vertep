## 1. Workspace setup

- [x] 1.1 Create the Cargo workspace with members `crates/vertep-core`, `crates/vertep-scxml`, `crates/vertep-dm-ecma`, `crates/vertep-rdf`, `crates/vertep-conformance`; pin the Rust toolchain; forbid `unsafe_code`; enable clippy lints workspace-wide
- [x] 1.2 Create the Lean package `lean/vertep-lean` with a pinned toolchain and LeanRDF pinned by revision; `lake build` runs in CI
- [x] 1.3 Add CI jobs: `cargo test`, `cargo build --no-default-features -p vertep-core --target thumbv7em-none-eabihf` (proves `no_std`), `lake build`, the conformance job and the nightly differential job

## 2. vertep-core: chart model

- [x] 2.1 Define the IR node types for the root, all state kinds, transitions, executable content, data items, invokes, params, content, donedata and extension nodes, with document positions (chart-model: Chart Document Structure, State Kinds, Transition Structure, Executable Content Model, Invoke Declaration, Datamodel Declaration, Extension Nodes)
- [x] 2.2 Implement deterministic generated identifiers for anonymous states that never collide with declared identifiers (State Identity)
- [x] 2.3 Implement document order and ordered collections, and structural chart equality that ignores source locations (Document Order, Chart Equality)
- [x] 2.4 Implement initial-state resolution for the root and compound states (Initial State Resolution)
- [x] 2.5 Implement well-formedness validation returning all violations with offending nodes (Well-Formedness Validation)
- [x] 2.6 Annotate the implementing items with `// @tg: implements:: [[openspec:chart-model#<Requirement>]]` for each of: Chart Document Structure, State Kinds, State Identity, Document Order, Transition Structure, Initial State Resolution, Executable Content Model, Invoke Declaration, Datamodel Declaration, Extension Nodes, Well-Formedness Validation, Chart Equality
- [x] 2.7 Write one test per chart-model scenario and annotate each with `// @tg: verifies:: [[openspec:chart-model#<Requirement>#<Scenario>]]` directly above the test

## 3. vertep-core: interpretation

- [x] 3.1 Build the compiled chart: dense state indices in document order, parent and ancestor tables, transition domains and default entries as bitsets
- [x] 3.2 Define the session value, inputs and the effect type (`Send`, `Cancel`, `Invoke`, `CancelInvoke`, `Forward`, `Log`, `SessionDone`) and the pure `start` and `step` functions (Pure Step Function)
- [x] 3.3 Implement session initialization with early and late binding and the initial macrostep (Session Initialization)
- [x] 3.4 Implement the macrostep loop with eventless-first ordering, the internal queue and the microstep budget (Macrostep Execution, Internal And External Queues)
- [x] 3.5 Implement event descriptor matching (Event Descriptor Matching)
- [x] 3.6 Implement transition selection and conflict removal per Appendix D (Optimal Enabled Transition Set)
- [x] 3.7 Implement exit and entry set computation, exit order, transition content and entry order, including internal transitions (Exit And Entry Order, Internal Transitions)
- [x] 3.8 Implement history recording and restoration with default transitions (History Restoration)
- [x] 3.9 Implement final states, `donedata` and `done.state` events, and session termination on top-level final or cancel (Final States And Done Events, Session Termination)
- [x] 3.10 Implement error handling for executable content: `error.execution`, `error.communication`, stopping the block (Execution Errors)
- [x] 3.11 Implement send and cancel effects with deterministic send identifiers and `#_internal` routing (Send And Cancel Effects)
- [x] 3.12 Implement the invoke lifecycle: start at macrostep end, cancel on exit, `finalize`, autoforward and `done.invoke` handling (Invoke Lifecycle)
- [x] 3.13 Annotate the implementing items with `// @tg: implements:: [[openspec:scxml-interpretation#<Requirement>]]` for each of: Pure Step Function, Session Initialization, Macrostep Execution, Event Descriptor Matching, Optimal Enabled Transition Set, Exit And Entry Order, Internal Transitions, History Restoration, Final States And Done Events, Internal And External Queues, Execution Errors, Send And Cancel Effects, Invoke Lifecycle, Session Termination
- [x] 3.14 Write one test per scxml-interpretation scenario and annotate each with `// @tg: verifies:: [[openspec:scxml-interpretation#<Requirement>#<Scenario>]]`
- [x] 3.15 Keep the kernel within the Aeneas-friendly subset: no trait objects inside the selection and entry/exit functions, no interior mutability, no async; document the boundary in the module docs

## 4. vertep-core and vertep-dm-ecma: datamodel

- [x] 4.1 Define the datamodel contract and the host-supplied registry; default to `null` (Datamodel Contract, Datamodel Registry)
- [x] 4.2 Implement system variables and `In()` shared by all datamodels (System Variables, In Predicate)
- [x] 4.3 Implement the `null` datamodel in the core (Null Datamodel)
- [x] 4.4 Implement the `ecmascript` datamodel on boa behind the `ecmascript` feature: global scope per session, JSON content, `foreach`, early and late binding (ECMAScript Datamodel, Data Binding Modes)
- [x] 4.5 Enforce guard purity with a read-only scope view; add the snapshot-compare backstop in debug builds (Guard Purity)
- [x] 4.6 Replace `Math.random` with a generator seeded from the session input and make `Date` read the input timestamp (Deterministic Evaluation)
- [x] 4.7 Implement datamodel snapshot and restore for both datamodels (Datamodel Snapshot)
- [x] 4.8 Record every datamodel request and answer to a tape when tracing is enabled, for the Lean oracle
- [x] 4.9 Annotate the implementing items with `// @tg: implements:: [[openspec:datamodel#<Requirement>]]` for each of: Datamodel Contract, Datamodel Registry, Guard Purity, System Variables, In Predicate, Null Datamodel, ECMAScript Datamodel, Data Binding Modes, Deterministic Evaluation, Datamodel Snapshot
- [x] 4.10 Write one test per datamodel scenario and annotate each with `// @tg: verifies:: [[openspec:datamodel#<Requirement>#<Scenario>]]`

## 5. vertep-scxml: SCXML format

- [x] 5.1 Implement the streaming reader for the SCXML namespace with source locations and recoverable error reporting (SCXML Document Parsing, Parse Error Reporting, Source Locations)
- [x] 5.2 Implement the writer, omitting generated identifiers unless referenced (SCXML Serialization)
- [x] 5.3 Implement the caller-supplied resolver for `src` attributes with no default I/O (External Content Resolution)
- [x] 5.4 Map the `https://w3id.org/vertep/vt#` namespace and foreign namespaces to extension nodes (Vertep Extension Namespace)
- [x] 5.5 Produce the loss report and strict mode (Declared Loss Report)
- [x] 5.6 Handle the `datamodel` attribute and unregistered datamodels (Datamodel Attribute Handling)
- [x] 5.7 Add round-trip property tests over generated charts and the conformance corpus (SCXML Round Trip)
- [x] 5.8 Annotate the implementing items with `// @tg: implements:: [[openspec:scxml-format#<Requirement>]]` for each of: SCXML Document Parsing, SCXML Serialization, SCXML Round Trip, Parse Error Reporting, Source Locations, External Content Resolution, Vertep Extension Namespace, Declared Loss Report, Datamodel Attribute Handling
- [x] 5.9 Write one test per scxml-format scenario and annotate each with `// @tg: verifies:: [[openspec:scxml-format#<Requirement>#<Scenario>]]`

## 6. vertep-rdf: chart RDF

- [x] 6.1 Write the `vt:` chart ontology module in Turtle with labels and definitions, and a test that every IR node kind has exactly one class (Chart Ontology Module)
- [x] 6.2 Implement the projection with node IRIs minted under the chart version IRI, no blank nodes, explicit `vt:order`, datamodel-typed expression literals and materialized `vt:ancestor` triples (IR To RDF Projection, Explicit Ordering, Executable Content As Literals)
- [x] 6.3 Write the SHACL Core shapes for the chart module and the empirical agreement test against IR validation on generated charts (SHACL Shapes For Charts)
- [x] 6.4 Implement reconstruction gated by SHACL validation, ignoring triples outside the projection (RDF To IR Reconstruction, Annotations Outside The Projection)
- [x] 6.5 Implement RDFC-1.0 canonical N-Quads, the SHA-256 hash and `urn:vertep:chart:sha256:<hex>` version IRIs; run the W3C RDFC test suite (Canonical Chart Hash, Chart Version IRI)
- [x] 6.6 Read and write Turtle, N-Triples, N-Quads and TriG in RDF 1.2, with TriG named graphs per chart version (RDF Serialization Formats)
- [x] 6.7 Add the round-trip identity property test over generated charts and the conformance corpus (Round Trip Identity)
- [x] 6.8 Annotate the implementing items with `// @tg: implements:: [[openspec:chart-rdf#<Requirement>]]` for each of: Chart Ontology Module, IR To RDF Projection, RDF To IR Reconstruction, Round Trip Identity, Explicit Ordering, Executable Content As Literals, SHACL Shapes For Charts, Canonical Chart Hash, Chart Version IRI, Annotations Outside The Projection, RDF Serialization Formats
- [x] 6.9 Write one test per chart-rdf scenario and annotate each with `// @tg: verifies:: [[openspec:chart-rdf#<Requirement>#<Scenario>]]`

## 7. vertep-lean: formal model

- [x] 7.1 Add the axiom check over main theorems and a no-`sorry` gate (Axiom Hygiene, Lean Package)
- [x] 7.2 Define the IR, `WellFormed` and a decision procedure with its correctness theorem (IR Model In Lean)
- [x] 7.3 Define the executable `step` with the datamodel as an answer-tape oracle and native `In()` (Executable Step Semantics)
- [x] 7.4 Prove step determinism (Step Determinism Theorem)
- [ ] 7.5 Prove that legal configurations are preserved (Configuration Well-Formedness Theorem)
- [x] 7.6 Prove that selected transition sets are enabled, conflict-free and optimal (Transition Selection Theorem)
- [ ] 7.7 Define the projection and reconstruction in Lean on LeanRDF's RDF 1.2 model and prove the round trip and canonical-form theorem (RDF Round Trip Theorem)
- [ ] 7.8 Transcribe the chart shapes into LeanRDF and prove `validate shapes (toRDF c) = conforms ↔ WellFormed c` (Shapes Equivalence Theorem)
- [x] 7.9 State the dynamic type-soundness theorem and the named M1 extension point (Type Soundness Statement)
- [x] 7.10 Build the oracle executable reading N-Quads, inputs and a tape and writing JSON Lines traces (Differential Oracle)
- [x] 7.11 Build `vt certify`, emitting `vt:certifiedBy` certificates with theorem names, toolchain, LeanRDF revision and checker hash (Chart Certificates)
- [ ] 7.12 Link each Lean declaration to its requirement with a `-- @tg: implements:: [[openspec:formal-model#<Requirement>]]` comment for every formal-model requirement, and annotate the Lean tests with `-- @tg: verifies:: [[openspec:formal-model#<Requirement>#<Scenario>]]`

## 8. vertep-conformance: conformance harness

- [x] 8.1 Write the import script that applies the pinned XSLT to the IRP `.txml` files, commits the `ecmascript` and `null` variants and their content hash, and classifies tests (IRP Suite Import)
- [x] 8.2 Implement the in-memory test host: multi-session SCXML Event I/O Processor, invoked `scxml` sessions, autoforward, `done.invoke`, virtual clock (Test Host)
- [x] 8.3 Implement pass/fail detection with step and virtual-time budgets (Pass Fail Protocol)
- [x] 8.4 Implement the allow-list with reasons, unexpected-pass failures and unknown-entry failures (Expected Deviation Allow List)
- [x] 8.5 Run all mandatory tests on the Rust core as a CI gate (Rust Core Conformance)
- [x] 8.6 Run the same tests on the Lean oracle with recorded tapes and compare traces (Lean Model Conformance)
- [x] 8.7 Run optional tests separately and list manual tests (Optional Test Reporting)
- [x] 8.8 Write the machine-readable report and the CI summary (Conformance Report)
- [x] 8.9 Implement chart and input generators with shrinking and a stored regression corpus; compare Rust and Lean traces (Randomized Differential Testing)
- [x] 8.10 Run the SCXML and RDF round-trip and shape checks over the corpus (Round Trip Corpus Checks)
- [x] 8.11 Annotate the implementing items with `// @tg: implements:: [[openspec:scxml-conformance#<Requirement>]]` for each of: IRP Suite Import, Pass Fail Protocol, Test Host, Rust Core Conformance, Lean Model Conformance, Expected Deviation Allow List, Optional Test Reporting, Conformance Report, Randomized Differential Testing, Round Trip Corpus Checks
- [x] 8.12 Write one test per scxml-conformance scenario and annotate each with `// @tg: verifies:: [[openspec:scxml-conformance#<Requirement>#<Scenario>]]`

## 9. Throwaway integration spike (deleted when M2 starts)

- [x] 9.1 DROPPED (see tg.md decisions#No throwaway journal spike), was THROWAWAY: create `spikes/journal-llm/` outside the workspace members with a README stating it is deleted at the start of M2 and nothing may depend on it
- [x] 9.2 DROPPED (see tg.md decisions#No throwaway journal spike), was THROWAWAY: run one session of the M0 kernel whose inputs and effects are appended to a tiramemsu file, and rebuild the session by replaying the journal
- [x] 9.3 DROPPED (see tg.md decisions#No throwaway journal spike), was THROWAWAY: add a stub decider state that sends the enabled event names to an LLM, records the answer as an input event, and replays without calling the model again
- [x] 9.4 DROPPED (see tg.md decisions#No throwaway journal spike), was THROWAWAY: write the findings (journal shape, effect handling, replay pitfalls, recorded-model-output size) into `tg.md` as notes for the M2 design

## 10. Documentation and traceability

- [x] 10.1 Record the design decisions D1 to D13 of this change in `tg.md/decisions.md` as `Decision` sections with `motivated_by` edges to the constraints and decisions they rest on
- [ ] 10.2 Add `constrains` edges from `tg.md/constraints.md` to the requirements they bind, for example `- constrains:: [[openspec:scxml-interpretation#Optimal Enabled Transition Set]] {strength: must}` from the SCXML interpretation algorithm constraint
- [ ] 10.3 Update the capability sections in `tg.md/capabilities.md` for chart-model, scxml-interpretation, scxml-format, datamodel, chart-rdf, formal-model and scxml-conformance with `specified_by` edges to their requirements, and keep them in the file's `openspec:` frontmatter
- [ ] 10.4 Update `tg.md/components.md` for vertep-core, vertep-scxml, vertep-dm-ecma, vertep-rdf, vertep-lean and vertep-conformance with `provides` edges to the requirements they implement and set their status
- [ ] 10.5 Run `tg check`, `tg ontology check` and `tg trace chart-model scxml-interpretation scxml-format datamodel chart-rdf formal-model scxml-conformance --gaps`, and close every gap they report
