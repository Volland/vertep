## 1. Type language in vertep-core

- [ ] 1.1 Add the type language data structures (primitives, records, tagged unions, lists, optional, named types, `value`) to `vertep-core` under `no_std + alloc`; annotate with `@tg: implements:: [[openspec:type-system#Primitive Types]]` and `@tg: implements:: [[openspec:type-system#Composite Types]]`
- [ ] 1.2 Implement the canonical value encoding and value checker reporting all errors with paths; annotate `@tg: implements:: [[openspec:type-system#Dynamic Value Boundary]]`
- [ ] 1.3 Implement recursive type validation (cycles only through list or optional); annotate `@tg: implements:: [[openspec:type-system#Recursive and Cyclic Type Definitions]]`
- [ ] 1.4 Implement event type declarations, descriptor unions and predefined system event types; annotate `@tg: implements:: [[openspec:type-system#Event Type Declarations]]`
- [ ] 1.5 Implement the chart type checker over context, `<data>`, `<send>`, `<param>`, guards and actions, collecting all diagnostics and refusing instantiation on error; annotate `@tg: implements:: [[openspec:type-system#Chart Type Checking]]` and `@tg: implements:: [[openspec:type-system#Opt-In Chart Typing]]`
- [ ] 1.6 Unit tests for every type-system scenario in 1.1–1.5, each test annotated with `@tg: verifies:: [[openspec:type-system#<Requirement>#<Scenario>]]` directly above it
- [ ] 1.7 Regression test: every M0 conformance chart runs untyped with an identical trace; annotate `@tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Untyped SCXML chart runs unchanged]]`

## 2. Type projections and content addressing

- [ ] 2.1 Serialize named types to RDF in the chart module and derive type IRIs with RDFC-1.0 in `vertep-rdf`; annotate `@tg: implements:: [[openspec:type-system#Named Content-Addressed Types]]`
- [ ] 2.2 Implement the SHACL Core projection of types in `vertep-rdf`; annotate `@tg: implements:: [[openspec:type-system#SHACL Projection]]`
- [ ] 2.3 Implement the JSON Schema 2020-12 projection; annotate `@tg: implements:: [[openspec:type-system#JSON Schema Projection]]`
- [ ] 2.4 Implement the CEL type environment projection; annotate `@tg: implements:: [[openspec:type-system#CEL Type Environment Projection]]`
- [ ] 2.5 Property tests generating types and valid and near-miss values, asserting checker, JSON Schema and SHACL agree; annotate with `@tg: verifies::` on the matching scenarios
- [ ] 2.6 Include types in chart version hashing and test surface-independent identity; annotate `@tg: verifies:: [[openspec:type-system#Types Survive Every Surface#Type change changes the chart version]]`

## 3. SCXML typing extension in vertep-scxml

- [ ] 3.1 Read and write `vt:type` on `<data>`, `<send>`, `<param>` and the `vt:types` element; annotate `@tg: implements:: [[openspec:type-system#Typed SCXML Attributes]]`
- [ ] 3.2 Tests for typed data, foreign-processor compatibility and unknown type names with suggestions, annotated with `@tg: verifies::`

## 4. YAML surface in vertep-yaml

- [ ] 4.1 Create `vertep-yaml` with an order-preserving, span-keeping YAML 1.2 parser; implement document header, version check and `datamodel` default; annotate `@tg: implements:: [[openspec:yaml-format#YAML Chart Document]]`
- [ ] 4.2 Lower states, hierarchy, parallel, final and history states; annotate `@tg: implements:: [[openspec:yaml-format#States and Hierarchy]]`
- [ ] 4.3 Lower `on`, `always` and transition forms; annotate `@tg: implements:: [[openspec:yaml-format#Transitions]]`
- [ ] 4.4 Lower executable content items, `invoke` and `context`; annotate `@tg: implements:: [[openspec:yaml-format#Executable Content]]` and `@tg: implements:: [[openspec:yaml-format#Invoke and Data in YAML]]`
- [ ] 4.5 Lower the `types` and `events` blocks; annotate `@tg: implements:: [[openspec:yaml-format#YAML Types Block]]`
- [ ] 4.6 Parse and type-check expression strings in the declared datamodel; annotate `@tg: implements:: [[openspec:yaml-format#YAML Uses Expression Strings of Its Datamodel]]`
- [ ] 4.7 Implement source-located diagnostics with dotted paths, reporting all errors; annotate `@tg: implements:: [[openspec:yaml-format#Source-Located Errors]]`
- [ ] 4.8 Implement the canonical YAML writer; annotate `@tg: implements:: [[openspec:yaml-format#Lossless YAML Round Trip]]`
- [ ] 4.9 Round-trip test over the whole SCXML conformance corpus (SCXML to YAML to IR) and byte-stability test, annotated with `@tg: verifies::`
- [ ] 4.10 Tests for every remaining yaml-format scenario, each annotated with `@tg: verifies::`

## 5. Rust datamodel in vertep-dm-rust

- [ ] 5.1 Create `vertep-dm-rust` implementing the `Datamodel` trait over a generated context struct with typed system variable accessors; annotate `@tg: implements:: [[openspec:rust-dsl#Rust Datamodel]]`
- [ ] 5.2 Implement the IRI-keyed function registry with duplicate rejection and unresolved-IRI errors; annotate `@tg: implements:: [[openspec:rust-dsl#Functions Registered by IRI]]`
- [ ] 5.3 Define guard and action function types (no emitter and shared context for guards; emitter for actions returning effects through the step output); annotate `@tg: implements:: [[openspec:rust-dsl#Pure Guards]]` and `@tg: implements:: [[openspec:rust-dsl#Actions Emit Effects as Events]]`
- [ ] 5.4 Tests, including trybuild compile-fail tests for guard mutation and emitter access, annotated with `@tg: verifies::`

## 6. CEL datamodel in vertep-dm-cel

- [ ] 6.1 Evaluate existing Rust CEL implementations for determinism, decimal support and limits; record the choice as a lat.md Decision in `tg.md/decisions.md` with `motivated_by` edges
- [ ] 6.2 Create `vertep-dm-cel` with datamodel selection and the `ctx`, `ev`, `session` and `In()` bindings; annotate `@tg: implements:: [[openspec:cel-datamodel#CEL Datamodel Selection]]` and `@tg: implements:: [[openspec:cel-datamodel#CEL Evaluation Scope]]`
- [ ] 6.3 Remove effectful functions from the environment and restrict mutation to `<assign>`; annotate `@tg: implements:: [[openspec:cel-datamodel#No Side Effects]]`
- [ ] 6.4 Implement step and value-size limits; annotate `@tg: implements:: [[openspec:cel-datamodel#Guaranteed Termination]]`
- [ ] 6.5 Type-check CEL against the projected environment at load; annotate `@tg: implements:: [[openspec:cel-datamodel#Static Type Checking of CEL]]`
- [ ] 6.6 Map runtime failures to `error.execution` and failed guards to false; annotate `@tg: implements:: [[openspec:cel-datamodel#Runtime Errors Raise Error Execution]]`
- [ ] 6.7 Define map ordering and exact decimals; annotate `@tg: implements:: [[openspec:cel-datamodel#Deterministic Evaluation]]`
- [ ] 6.8 Implement the CEL-only loading policy; annotate `@tg: implements:: [[openspec:cel-datamodel#Safe Datamodel for Generated Charts]]`
- [ ] 6.9 Implement `<data>`, `<assign>`, `<donedata>`, `<content>`, `<param>`, `<foreach>`, `<if>`, `<log>` semantics; annotate `@tg: implements:: [[openspec:cel-datamodel#SCXML Datamodel Contract]]`
- [ ] 6.10 Run the SCXML conformance tests that are datamodel-neutral under CEL (transformed from the ECMAScript templates) and record deviations in the allow-list
- [ ] 6.11 Native versus WebAssembly determinism test over an expression corpus, and tests for every cel-datamodel scenario, each annotated with `@tg: verifies::`

## 7. Macros in vertep-macros

- [ ] 7.1 Create `vertep-macros`; implement the `statechart!` token grammar and lowering to IR with spans; annotate `@tg: implements:: [[openspec:rust-dsl#Statechart Macro]]`
- [ ] 7.2 Implement `include_chart!` for `.scxml` and `.yaml` with manifest-relative paths and rebuild tracking; annotate `@tg: implements:: [[openspec:rust-dsl#Include Chart Macro]]`
- [ ] 7.3 Generate the state enum, event enum, context struct, IR constant, version IRI constant and `bind()`; annotate `@tg: implements:: [[openspec:rust-dsl#Generated Typed API]]` and `@tg: implements:: [[openspec:type-system#Rust Projection]]`
- [ ] 7.4 Emit structural diagnostics with spans and suggestions; annotate `@tg: implements:: [[openspec:rust-dsl#Compile-Time Structural Checks]]`
- [ ] 7.5 Implement reachability and unhandled-event diagnostics with the error-promotion attribute; annotate `@tg: implements:: [[openspec:rust-dsl#Compile-Time Reachability and Coverage Diagnostics]]`
- [ ] 7.6 Check guard and action signatures through generated trait bounds; annotate `@tg: implements:: [[openspec:rust-dsl#Guard and Action Signature Checking]]`
- [ ] 7.7 trybuild pass and compile-fail suites for every rust-dsl scenario, each annotated with `@tg: verifies::`
- [ ] 7.8 Equivalence test: the design's `order` chart via `statechart!`, `include_chart!("order.scxml")` and `include_chart!("order.yaml")` (rust datamodel variant) yields the same IR, API and version IRI

## 8. Command line in vertep-cli

- [ ] 8.1 Create `vertep-cli` with the `vt` binary and `typecheck`, `schema`, `validate`, `hash` and `certify` subcommands; annotate `@tg: implements:: [[openspec:type-system#Command Line Type Tools]]`
- [ ] 8.2 Implement `vt convert` with surface detection and lossy-conversion refusal; annotate `@tg: implements:: [[openspec:yaml-format#Command Line Conversion]]`
- [ ] 8.3 Integration tests for each CLI scenario, annotated with `@tg: verifies::` at level integration

## 9. Lean formal model

- [ ] 9.1 Add the type language, `HasType` and the executable checker to `vertep-lean`; prove `check T v = true ↔ HasType v T`
- [ ] 9.2 Define the SHACL projection in Lean and prove agreement using LeanRDF's `validate_conforms`
- [ ] 9.3 Define the JSON Schema projection in Lean and prove agreement for the supported keyword subset
- [ ] 9.4 State and prove chart type soundness for `step`: well-typed chart, configuration and input events yield well-typed context and output events
- [ ] 9.5 Extend the differential oracle: the Lean checker and the Rust checker give the same verdict and diagnostics count on generated charts and values in CI
- [ ] 9.6 Pin the axioms of every new theorem in tests (no `sorry`, only `propext`, `Quot.sound`, `Classical.choice`)

## 10. Documentation and graph

- [ ] 10.1 Update the lat.md sections [[capabilities#Type system]], [[capabilities#Rust DSL]], [[capabilities#YAML format]] and [[capabilities#CEL datamodel]] with `specified_by` edges to the new requirements, and keep `type-system`, `rust-dsl`, `yaml-format` and `cel-datamodel` in the `openspec:` frontmatter of `tg.md/capabilities.md`
- [ ] 10.2 Record design decisions 1–7 of design.md as `Decision` sections in `tg.md/decisions.md` with `motivated_by` edges to [[decisions#One IR type language]], [[decisions#Pluggable datamodel]] and [[constraints#Proc macros run at compile time]]; add `- constrains:: [[openspec:rust-dsl#Compile-Time Structural Checks]] {strength: must}` to [[constraints#Proc macros run at compile time]]
- [ ] 10.3 Set `status: in-progress` then `stable` on the M1 components in `tg.md/components.md` and add `provides` edges from each crate to the requirements it implements
- [ ] 10.4 Run `tg check`, `tg ontology check` and `tg trace type-system rust-dsl yaml-format cel-datamodel --gaps`, and close every gap they report
