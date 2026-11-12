## 1. Lean oracle fixes (from M0 differential testing)

- [x] 1.1 Make `vertep-oracle` terminate on `crates/vertep-conformance/regressions/5cbfa3338d26b03f.json` and charge idle internal events against the microstep budget exactly like `vertep-core`
- [x] 1.2 Emit the partial trace together with the budget error, and drop the harness's special case for budget exhaustion
- [x] 1.3 `cargo test -p vertep-conformance --release --features differential -- --ignored differential` passes

## 2. M0 theorems (from m0-scxml-core 7.5, 7.7, 7.8, 7.12)

- [x] 2.1 Prove the RDF Round Trip Theorem: `fromRDF (project c base) = ok (c, base)` for well-formed charts
  - Done (2026-11-11): `Vertep.Rdf.fromRDF_project` (`VertepLean/Rdf/RoundTrip/Chart.lean`), for every base. Built on `dec_state` (states and their subtrees, `RoundTrip/State.lean`), `decode_project` (the chart node, the unique `vt:Chart` node via `NT`, the base IRI via `baseOf`) and the decoders of `RoundTrip/Nodes.lean` (data, invokes, done data, handlers, `<initial>`); the closed-world check holds because the graph is the projection. Uses from `WellFormed` only unique identifiers and known targets. Axioms pinned: `propext`, `Classical.choice`, `Quot.sound`. `Pending/RoundTrip.lean` is retired.
- [ ] 2.2 Prove the Configuration Well-Formedness Theorem for `start` and `step`
  - Not started (2026-11-11): the statement and the list of missing lemmas (compile correctness, exit and entry lemmas, the history invariant, fuel adequacy, invariants through `exitStates`/`enterStates`/`microstep`/`macroLoop`) are in `Pending/ConfigWF.lean`; no counterexample is known under the current `WellFormed`.
- [ ] 2.3 Prove the Shapes Equivalence Theorem: `validate shapes (toRDF c) = conforms ↔ WellFormed c`
  - Not started (2026-11-11): see `Pending/ShapesEquiv.lean`. The capture and lookup lemmas of `VertepLean/Rdf/RoundTrip/` now cover the decoder's paths on `project c base`; the shape paths (`vt:ancestor`, `vt:scope`, `vt:choice`, `vt:historyDefault`) and the shapes as a Lean term remain.
- [ ] 2.4 Annotate every formal-model declaration and test with `@tg: implements::` and `@tg: verifies::` and pin each theorem's axioms
  - Partial (2026-11-11): every theorem in `VertepTests/Axioms.lean` `mainTheorems` (65, all on `propext`, `Classical.choice`, `Quot.sound` or fewer) carries an `@tg: implements::` line, and every formal-model scenario with a proof has a `@tg: verifies::` test (`tg trace formal-model --gaps`: 13/14 implemented, 12/14 verified). Remaining: the Configuration Well-Formedness and Shapes Equivalence theorems and their scenarios, which wait on 2.2 and 2.3.

## 3. CEL conformance (from m1-typed-authoring 6.10)

- [x] 3.1 Generate the CEL variant of the datamodel-neutral IRP tests with `confCel.xsl`, record excluded tests with reasons, and gate them in the harness with an allow-list

## 4. Lean type language (from m1-typed-authoring 9.1 to 9.6)

- [x] 4.1 Add the type language, `HasType` and the executable checker; prove `check T v = true ↔ HasType v T`
  - Mirror the Rust checker on non-contractive recursion (decision "Type checking stops on non-contractive recursion"): declarations recursing only through `optional` (`type A = optional<A>`, `type B = optional<C>; type C = optional<B>`, `type D = optional<optional<D>>`) are valid, as the spec guards recursion by list or optional; `HasType` is the least fixed point, so their only value is `null`; the executable `check` must terminate on every declaration block `validate` accepts. Rust stops when a named type repeats at the same value position (the set of names under expansion is cleared on descent into a list item, record field or union payload) and reports a mismatch; the Lean `check` needs the same rule, or a measure (value size, then names not yet expanded at this position), for its termination proof. Test case: `recursion_through_optional_only_terminates` in `crates/vertep-core/src/types/tests.rs`.
  - Done (2026-11-11): `VertepLean/Types/Basic.lean`, `Types/Check.lean`; `Vertep.Types.check_iff : check D [] t v = true ↔ HasType D v t` for every declaration block (no `validate` hypothesis needed: `check` terminates on alias cycles too), via `check_sound` and `Chain.dedup` (loop cutting). Tests in `VertepTests/Types.lean` mirror the Rust value-checker tests, and `optional_loop_values` proves `"x"` is no value of `type A = optional<A>`.
- [ ] 4.2 Define the SHACL projection of types and prove agreement with LeanRDF's validator
  - Open (2026-11-11): not started. Needs the value encoding of `vertep-rdf/src/types/value.rs` as a LeanRDF graph (fresh blank nodes for objects, RDF collections for lists), the shapes of `shapes.rs` (closed blank nodes, `sh:or`, `sh:node` recursion through named types, `rdf:rest*/rdf:first` member paths, `sh:pattern`), and a proof against LeanRDF's `Conforms` via `results_nil`.
- [x] 4.3 Define the JSON Schema projection and prove agreement for the supported keyword subset
  - Done (2026-11-11): `VertepLean/Types/JsonSchema.lean` mirrors `json_schema.rs`; `JsonSchema.validate_proj` (validate of the projection equals `check`) and `JsonSchema.jsonSchema_agrees : validate (defsOf D) [] (proj D t) v = true ↔ HasType D v t` for JSON values (no `undefined`) and blocks whose unions have distinct tags. `pattern` is read as the lexical rule its regular expression encodes; the regex-to-rule equivalence is not proven.
- [ ] 4.4 Prove chart type soundness for `step`
  - Open (2026-11-11): not started. The Lean `step` has no expression typer: every value it writes to a location comes from the untyped answer tape (`evalValue`, `evalContent`, `foreachItems` answers), from invoker data, or is generated (`idlocation` strings, `undefined` for a bare `assign`, `foreach` indices). Stating `TypedStepSoundness` for `HasType` needs the static typing of the chart in the model (the type the Rust chart checker gives each expression and location), a `TapeTyped` that types each answer by its request's expression, and threading a typing proof through each `assignValue`/`bindVar` of `Step.lean` the way `requests_ok` threads `LocOk`.
- [ ] 4.5 Mirror the `vt-types` projection so `vt certify` certifies typed charts, and extend the differential oracle to the type checker
  - Partial (2026-11-11): the differential oracle is extended: `lake exe vertep-types --cases CASES.jsonl` (`VertepLean/Types/Oracle.lean`) and `crates/vertep-conformance/tests/types_differential.rs` (4,000 generated blocks, types and values incl. optional-only recursion, unknown names and alias cycles; same verdict as `Declarations::check`; diagnostics counts are not compared). Remaining: reading `vt-types` triples in the Lean RDF projection and reconstruction (`VertepLean/Rdf/`) so `vt certify` accepts typed charts.
- [ ] 4.6 Pin the axioms of every new theorem
  - Partial (2026-11-11): `Vertep.Types.check_iff`, `check_sound`, `Chain.dedup`, `JsonSchema.validate_proj` and `JsonSchema.jsonSchema_agrees` are in `mainTheorems` of `VertepTests/Axioms.lean` (`propext`, `Classical.choice`, `Quot.sound`; `Chain.dedup` only `propext`). Remaining: the theorems of 4.2 and 4.4.

## 5. Lean journal protocol (from m2-actor-runtime 12.1 to 12.6)

- [x] 5.1 Model journal entries, compare-and-set append, deduplication, the outbox and replay
- [x] 5.2 Prove replay determinism
- [x] 5.3 Prove deduplication idempotence
- [x] 5.4 Prove the outbox property
- [x] 5.5 Prove compare-and-set single activation
- [x] 5.6 Add Rust property tests of crashes, redeliveries and concurrent activations with the Lean model as oracle
  - `crates/vertep-conformance/tests/journal_protocol.rs` with `vertep-journal`. Found: two live nodes whose durable mailboxes share one store lose messages (`StoreMailbox` row IRIs came from each mailbox's own sequence); fixed 2026-11-11 by row IRIs that carry the envelope's digest, and `journal_protocol_shared_durable_mailboxes_lose_no_message` now runs. Also fixed: the Rust outbox mark advanced over sends a transport refused; it now moves only over delivered sends, as the model's `outbox_safe` assumes.
