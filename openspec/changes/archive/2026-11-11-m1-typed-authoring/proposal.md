## Why

After M0, charts run with correct SCXML semantics, but every surface is untyped: events and context are dynamic values, and guards and actions are strings an engine interprets. Vertep's promise of a "type-provable" DSL, of typed LLM structured output in M3, and of charts written by LLMs that are safe to run all depend on one shared type language and on typed authoring surfaces. Without it, the macro DSL, YAML, SHACL shapes and JSON Schema would each invent their own schema and drift apart.

## What Changes

- Add an algebraic type language to the chart IR: primitives (bool, int, decimal, string, datetime, IRI), records, tagged unions, lists, optional values and named types that are content-addressed like charts.
- Add a chart type checker for event payloads, context, guard and action signatures, `<send>` payloads and `<data>` declarations, with typing opt-in per chart and a dynamic value fallback for untyped charts.
- Project every IR type to Rust types, JSON Schema, SHACL shapes and a CEL type environment, with the guarantee that a value accepted by one projection is accepted by all.
- Add the `vt:type` extension attribute to SCXML so XML charts carry the same types.
- Add the `statechart!` procedural macro and `include_chart!` for SCXML and YAML files, both generating the same typed Rust API, with compile-time diagnostics for unknown targets, unreachable states, signature mismatches and unhandled events.
- Add the `rust` datamodel: typed context and named guard and action functions registered by IRI, with guards required to be pure.
- Add the YAML chart surface, one to one with the IR, including a `types:` block and source-located errors.
- Add the CEL datamodel: side-effect-free, terminating, type-checked before a chart runs, raising `error.execution` on runtime failures; the datamodel for YAML charts and for LLM-written charts.
- Add the `vt` command line tool: convert between SCXML, YAML and RDF, validate, typecheck, hash and certify charts.
- Extend the Lean formal model with the type language and a type soundness theorem.

## Capabilities

### New Capabilities

- `type-system`: the IR type language, chart type checking, projections to Rust, JSON Schema, SHACL and CEL, opt-in typing and the dynamic fallback, and `vt:type` in SCXML.
- `rust-dsl`: the `statechart!` and `include_chart!` macros, their generated typed API and compile-time diagnostics, and the `rust` datamodel.
- `yaml-format`: the YAML surface for charts, its mapping to the IR, its `types:` block, round trip and error reporting.
- `cel-datamodel`: the CEL datamodel for guards, assignments and expressions, its type checking, determinism and error behavior.

### Modified Capabilities

None. M0 capabilities (chart model, datamodel, SCXML format, chart RDF, formal model) are extended through these new capabilities rather than by changing their requirements.

## Impact

- **tg sections:** [[capabilities#Type system]], [[capabilities#Rust DSL]], [[capabilities#YAML format]], [[capabilities#CEL datamodel]]; components [[components#vertep-core]], [[components#vertep-macros]], [[components#vertep-dm-rust]], [[components#vertep-yaml]], [[components#vertep-dm-cel]], [[components#vertep-rdf]], [[components#vertep-cli]], [[components#vertep-lean]]; milestone [[roadmap#M1 Typed authoring]].
- **Binding decisions:** [[decisions#One IR type language]], [[decisions#Pluggable datamodel]], [[decisions#IR is the hub]], [[decisions#Lean verification bridge]], [[decisions#Modular vt ontology on RDF 1.2]], [[decisions#No-std core and tokio shell]].
- **Binding constraints:** [[constraints#Proc macros run at compile time]], [[constraints#RDF is open-world]], [[constraints#SCXML interpretation algorithm]].
- **New crates:** `vertep-macros`, `vertep-dm-rust`, `vertep-yaml`, `vertep-dm-cel`, `vertep-cli`. Extended: `vertep-core` (type language and checker), `vertep-rdf` (SHACL projection of types), `vertep-scxml` (`vt:type`), `vertep-lean` (type soundness).
- **New dependencies:** a YAML parser with source spans, a CEL parser and evaluator (or an in-house CEL subset), `syn`/`quote`/`proc-macro2` for macros, a JSON Schema validator for tests.
- **Downstream:** the M3 deciders capability consumes the JSON Schema projection for LLM structured output; the M3 plan charts capability requires the CEL datamodel; M2 runtime ontology reuses the SHACL projection.
