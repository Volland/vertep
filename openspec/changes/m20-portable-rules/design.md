## Context

`vertep-rules` runs rule programs through tiramemsu's `elpi` feature, which links `libleanelpi` (Lean runtime and the ELPI interpreter). M14 decided the native engine is optional and that a build without it refuses rules (`rules-unavailable`); it left a Rust evaluator for a later milestone. M16 defined the rules profile and a lexical profile check (`profile.rs`), and its oracle test (`tests/oracle.rs`) shows ELPI and `tm-datalog` agree on a Horn join. The browser build already carries a tiramemsu shard (`tm-wasm`, SQLite in OPFS), so the statements a rule reads exist there; only the evaluator is missing.

## Goals / Non-Goals

**Goals:**
- Rule decisions in the browser, on Windows and in Python, for programs inside the portable profile, with the same records as native decisions.
- A conformance suite that fails CI when the two engines disagree on conclusions or stated support.
- Bundles made on one engine verifiable on the other.

**Non-Goals:**
- Negation, `findall`, higher-order clauses or effects in the portable profile.
- Replacing the native engine; it stays the reference and the only engine for programs outside the portable profile.
- Making fuel numbers comparable across engines.

## Decisions

### P1. The portable profile is the Horn part of the rules profile, checked by a parser

The portable profile is a strict subset of M16's rules profile: clauses whose heads are `tm.conclude (triple S P O) Support` (or `tm.violation` for invariants), whose bodies are conjunctions of `tm.triple S P O E`, comparisons (`<`, `=<`, `>`, `>=`, `=`, `\=`) and `is` over `+ - * /` on numbers, with ground strings, numbers and `(iri "...")`. The check parses clauses into a small AST, unlike M16's lexical check, because the portable engine must execute exactly what it accepts. A program the parser rejects is outside the portable profile, with the first clause and the construct named.

Alternative rejected: reuse the lexical check. It is conservative about what it accepts, which is right for marking decisions `unchecked` and wrong for choosing an engine.

### P2. Evaluate by translating to `tm-datalog`, behind a spike

The portable engine translates a parsed program into `tm-datalog`'s program form and runs it on the same view as of the decision's transaction. `tm-datalog` already has semi-naive evaluation, limits and provenance. The spike (task 1) settles two questions before the translation is committed: whether `tm-datalog` exposes a statement's eid as a term so that stated support can be bound, and whether it builds for `wasm32-unknown-unknown` inside `tm-wasm`. If either fails, the fallback is a direct top-down evaluator in `vertep-rules` over `tm.triple` lookups; decision programs are small and mostly non-recursive, so it does not need tabling for the first release.

### P3. Stated support is checked the same way in both engines

Support is what the program states, not what provenance would infer. The portable engine keeps the set of statements a run read and refuses support outside it, with the same reason `forged-support`, so a program behaves the same on both engines. Provenance from `tm-datalog` is used only as a cross-check in the conformance suite.

### P4. The engine is recorded; replay keeps the family, verification may cross

A derivation's `engine` field becomes `portable/<crate version>` or `lean-elpi/<library version>`. Replay of a journal uses the recorded engine family and reports `engine-unavailable` if it is not built in. Bundle verification is a check of conclusion and support, not of fuel, so for a portable program either engine may verify it; the report says `cross-engine` when the engines differ. Fuel is counted in each engine's own steps; the budget in `RulePolicy` applies to whichever runs, and fuel used is compared only between runs of the same engine.

Alternative rejected: one shared step count. The engines do different work per conclusion, and a shared count would have to be emulated in one of them.

### P5. Selection is per program and decided at load

`RuleDecider::new` picks the engine when it loads the program: native when the `native` feature is on (unless `RulePolicy` asks for portable), portable when the program is inside the portable profile, `rules-unavailable` otherwise. A program edit is checked again when it supersedes the live one; an edit that leaves the portable profile on a portable-only build is refused at `put_program`, so a running decider never meets a program it cannot run.

## Risks / Trade-offs

- Two engines can drift. Mitigation: the conformance suite runs every decision vector and pseudo-random graphs on both, on every CI run that has the native library, and the Lean certificates of M16 check either engine's derivation.
- `tm-datalog` semantics of numbers and strings may differ from ELPI's (integer versus float, string comparison). Mitigation: the portable profile fixes the comparison and arithmetic semantics, and the suite includes boundary vectors (`43` versus `43.0`, large amounts).
- Wasm size. The portable engine adds to `@vertep/web`. Mitigation: it sits behind a `rules` feature of `vertep-wasm`, measured against the bundle size budget.

## Migration Plan

Existing derivations record `engine` as the native library version; they are read as `lean-elpi/<version>`. No journal is rewritten. The `native` feature keeps its meaning.

## Open Questions

- Should `RulePolicy` let an operator require the native engine for audit reasons even for portable programs? The default proposed is yes, as an option, off by default.
- Does the portable engine also run `tm.violation` invariants (M15) in the first release, or only deciders? Proposed: deciders and invariants, since invariants are Horn too; derived facts (M15 `derived-facts`) follow if the spike shows materialization is the same translation.
