## 1. Lean oracle fixes (from M0 differential testing)

- [x] 1.1 Make `vertep-oracle` terminate on `crates/vertep-conformance/regressions/5cbfa3338d26b03f.json` and charge idle internal events against the microstep budget exactly like `vertep-core`
- [x] 1.2 Emit the partial trace together with the budget error, and drop the harness's special case for budget exhaustion
- [x] 1.3 `cargo test -p vertep-conformance --release --features differential -- --ignored differential` passes

## 2. M0 theorems (from m0-scxml-core 7.5, 7.7, 7.8, 7.12)

- [ ] 2.1 Prove the RDF Round Trip Theorem: `fromRDF (project c base) = ok (c, base)` for well-formed charts
  - Partial: `VertepLean/Rdf/RoundTrip/` proves `percentEncode_inj`, `ordered_eq`, capture of a node's triples (`Cap.kid`), and decoding of executable content and transitions (`dec_exec`, `dec_transition`), axioms pinned. Remaining: decoding states (the `state` recursion with its depth fuel), the chart node, the base IRI, the unique `vt:Chart` node, and the closed-world check on `project c base` itself.
- [ ] 2.2 Prove the Configuration Well-Formedness Theorem for `start` and `step`
- [ ] 2.3 Prove the Shapes Equivalence Theorem: `validate shapes (toRDF c) = conforms ↔ WellFormed c`
- [ ] 2.4 Annotate every formal-model declaration and test with `@tg: implements::` and `@tg: verifies::` and pin each theorem's axioms

## 3. CEL conformance (from m1-typed-authoring 6.10)

- [x] 3.1 Generate the CEL variant of the datamodel-neutral IRP tests with `confCel.xsl`, record excluded tests with reasons, and gate them in the harness with an allow-list

## 4. Lean type language (from m1-typed-authoring 9.1 to 9.6)

- [ ] 4.1 Add the type language, `HasType` and the executable checker; prove `check T v = true ↔ HasType v T`
  - Mirror the Rust checker on non-contractive recursion (decision "Type checking stops on non-contractive recursion"): declarations recursing only through `optional` (`type A = optional<A>`, `type B = optional<C>; type C = optional<B>`, `type D = optional<optional<D>>`) are valid, as the spec guards recursion by list or optional; `HasType` is the least fixed point, so their only value is `null`; the executable `check` must terminate on every declaration block `validate` accepts. Rust stops when a named type repeats at the same value position (the set of names under expansion is cleared on descent into a list item, record field or union payload) and reports a mismatch; the Lean `check` needs the same rule, or a measure (value size, then names not yet expanded at this position), for its termination proof. Test case: `recursion_through_optional_only_terminates` in `crates/vertep-core/src/types/tests.rs`.
- [ ] 4.2 Define the SHACL projection of types and prove agreement with LeanRDF's validator
- [ ] 4.3 Define the JSON Schema projection and prove agreement for the supported keyword subset
- [ ] 4.4 Prove chart type soundness for `step`
- [ ] 4.5 Mirror the `vt-types` projection so `vt certify` certifies typed charts, and extend the differential oracle to the type checker
- [ ] 4.6 Pin the axioms of every new theorem

## 5. Lean journal protocol (from m2-actor-runtime 12.1 to 12.6)

- [x] 5.1 Model journal entries, compare-and-set append, deduplication, the outbox and replay
- [x] 5.2 Prove replay determinism
- [x] 5.3 Prove deduplication idempotence
- [x] 5.4 Prove the outbox property
- [x] 5.5 Prove compare-and-set single activation
- [x] 5.6 Add Rust property tests of crashes, redeliveries and concurrent activations with the Lean model as oracle
  - `crates/vertep-conformance/tests/journal_protocol.rs` with `vertep-journal`. Found: two live nodes whose durable mailboxes share one store lose messages (`StoreMailbox` row IRIs came from each mailbox's own sequence); fixed 2026-11-11 by row IRIs that carry the envelope's digest, and `journal_protocol_shared_durable_mailboxes_lose_no_message` now runs. Also fixed: the Rust outbox mark advanced over sends a transport refused; it now moves only over delivered sends, as the model's `outbox_safe` assumes.
