## 1. Spike (throwaway, go/no-go)

- [ ] 1.1 Check whether `tm-datalog` exposes a statement's eid as a term, so stated support can be bound; record the answer in `design.md` (P2)
- [ ] 1.2 Build `tm-datalog` for `wasm32-unknown-unknown` inside `tm-wasm` and for Windows; record size added to `@vertep/web`
- [ ] 1.3 Run the credit policy through a hand translation and compare conclusions and support with ELPI; record go (translation) or the fallback (direct evaluator)

## 2. Graph first

- [ ] 2.1 Add milestone `M20 Portable rules` to `tg.md/roadmap.md`, `precedes::` from M14 and M16
- [ ] 2.2 Add Capability `Rule engines` to `tg.md/capabilities.md` and list `rule-engines` in its `openspec:` frontmatter
- [ ] 2.3 Record P1-P5 as `Decision` sections; mark `The native rules engine is optional behind a feature` as refined by P5
- [ ] 2.4 Update `components#vertep-rules` with the portable engine and its `realizes::` edges
- [ ] 2.5 Add a section to `views/rules.md` with a Mermaid diagram of engine selection

## 3. Portable profile

- [ ] 3.1 Parser for the portable profile into a clause AST, with the first non-portable clause and construct named
- [ ] 3.2 Vectors: credit policy portable; `findall`, negation, effects and higher-order clauses not portable

## 4. Portable engine

- [ ] 4.1 Evaluator over a tiramemsu view as of a transaction (translation to `tm-datalog`, or the direct evaluator per the spike)
- [ ] 4.2 Read set and the `forged-support` check
- [ ] 4.3 Fuel in engine steps under `RulePolicy`; `fuel-exhausted` refusal
- [ ] 4.4 `tm.violation` for enforced invariants (M15) on the portable engine

## 5. Selection and records

- [ ] 5.1 Engine selection in `RuleDecider::new` and `put_program`; `RulePolicy` option to require native or portable
- [ ] 5.2 `engine` recorded as `portable/<v>` or `lean-elpi/<v>`; old derivations read as `lean-elpi`
- [ ] 5.3 Replay keeps the engine family, `engine-unavailable` otherwise
- [ ] 5.4 Bundle verification across engines, report `cross-engine`

## 6. Agreement

- [ ] 6.1 Conformance suite: every rule-decider vector on both engines, decisions, support and refusal reasons equal
- [ ] 6.2 Pseudo-random graphs (seeded) on both engines; numeric and string boundary vectors
- [ ] 6.3 CI: suite on Linux with the native library; portable vectors on the wasm and Windows jobs

## 7. Bindings and docs

- [ ] 7.1 `vertep-wasm` feature `rules`: decider, `verifyBundle`, counterfactuals; TypeScript types; bundle size budget checked
- [ ] 7.2 Python: rules decider, bundle verification and counterfactuals; a runnable example
- [ ] 7.3 Update `site/docs/rules*.html`, `browser.html`, `python.html`, tiers and the claims audit; `examples/credit-decision` runs in the browser
