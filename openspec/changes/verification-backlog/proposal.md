## Why

Milestones M0, M1 and M2 are complete in code: every requirement is implemented and verified by tests, the W3C SCXML suite passes on the Rust core and replays identically on the Lean oracle, and Rust and Lean agree on generated charts. What remains is formal proof work, one conformance variant and one oracle bug. Keeping them in the milestone changes would block archiving finished work, so they move here as one tracked backlog.

## What Changes

- Prove the three pending M0 theorems now stated in `lean/vertep-lean/Pending/`: configuration well-formedness, RDF round trip and shapes equivalence, and complete their `@tg:` annotations.
- Fix the Lean oracle that does not terminate on the stored regression `crates/vertep-conformance/regressions/5cbfa3338d26b03f.json`, mirror the microstep budget that also charges idle internal events, and emit partial traces on budget exhaustion.
- Run the datamodel-neutral W3C tests under the CEL datamodel (carried from M1 task 6.10).
- Model the M1 type language in Lean and prove checker, SHACL and JSON Schema agreement and chart type soundness; mirror the type projection so typed charts can be certified (carried from M1 group 9).
- Model the M2 journal protocol in Lean and prove replay determinism, deduplication idempotence, the outbox property and compare-and-set single activation, with a Rust property-test oracle (carried from M2 group 12).

No requirement changes: every item implements or verifies requirements already in `openspec/specs/` (formal-model, cel-datamodel, type-system, event-journal, storage).

## Impact

- `lean/vertep-lean` gains proofs and models; `crates/vertep-conformance` gains the CEL suite variant and journal-protocol property tests.
- The `formal-model` requirements Configuration Well-Formedness Theorem, RDF Round Trip Theorem and Shapes Equivalence Theorem become verified when their tasks close.
