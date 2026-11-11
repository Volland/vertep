## 1. Lean oracle fixes (from M0 differential testing)

- [ ] 1.1 Make `vertep-oracle` terminate on `crates/vertep-conformance/regressions/5cbfa3338d26b03f.json` and charge idle internal events against the microstep budget exactly like `vertep-core`
- [ ] 1.2 Emit the partial trace together with the budget error, and drop the harness's special case for budget exhaustion
- [ ] 1.3 `cargo test -p vertep-conformance --release --features differential -- --ignored differential` passes

## 2. M0 theorems (from m0-scxml-core 7.5, 7.7, 7.8, 7.12)

- [ ] 2.1 Prove the RDF Round Trip Theorem: `fromRDF (project c base) = ok (c, base)` for well-formed charts
- [ ] 2.2 Prove the Configuration Well-Formedness Theorem for `start` and `step`
- [ ] 2.3 Prove the Shapes Equivalence Theorem: `validate shapes (toRDF c) = conforms ↔ WellFormed c`
- [ ] 2.4 Annotate every formal-model declaration and test with `@tg: implements::` and `@tg: verifies::` and pin each theorem's axioms

## 3. CEL conformance (from m1-typed-authoring 6.10)

- [ ] 3.1 Generate the CEL variant of the datamodel-neutral IRP tests with `confCel.xsl`, record excluded tests with reasons, and gate them in the harness with an allow-list

## 4. Lean type language (from m1-typed-authoring 9.1 to 9.6)

- [ ] 4.1 Add the type language, `HasType` and the executable checker; prove `check T v = true ↔ HasType v T`
- [ ] 4.2 Define the SHACL projection of types and prove agreement with LeanRDF's validator
- [ ] 4.3 Define the JSON Schema projection and prove agreement for the supported keyword subset
- [ ] 4.4 Prove chart type soundness for `step`
- [ ] 4.5 Mirror the `vt-types` projection so `vt certify` certifies typed charts, and extend the differential oracle to the type checker
- [ ] 4.6 Pin the axioms of every new theorem

## 5. Lean journal protocol (from m2-actor-runtime 12.1 to 12.6)

- [ ] 5.1 Model journal entries, compare-and-set append, deduplication, the outbox and replay
- [ ] 5.2 Prove replay determinism
- [ ] 5.3 Prove deduplication idempotence
- [ ] 5.4 Prove the outbox property
- [ ] 5.5 Prove compare-and-set single activation
- [ ] 5.6 Add Rust property tests of crashes, redeliveries and concurrent activations with the Lean model as oracle
