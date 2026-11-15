## 1. Spike (throwaway, go/no-go)

- [x] 1.1 Enable tiramemsu `elpi` and `native` in a scratch branch of `vertep-tiramemsu` and build against `libleanelpi` on macOS and Linux
- [ ] 1.2 Write the credit-decision policy as one ELPI program node and a decider that runs it over the offered candidates (policy and run done in the spike; the `Decider` wiring remains for group 4)
- [x] 1.3 Check the refused-answer case: the journal alone shows the derivation, and `as_of` replays the previous program
- [x] 1.4 Record the go/no-go and the numbers (latency per decision, library size) in `design.md`; delete the spike code (the spike was never in the repository)

## 2. Graph first

- [x] 2.1 Add milestone `M14 Rules decider` to `tg.md/roadmap.md`, `precedes::` from M12 and M13
- [x] 2.2 Add Capabilities `Rule programs` and `Rule decider` to `tg.md/capabilities.md` and list both in its `openspec:` frontmatter
- [x] 2.3 Record R1-R6 as `Decision` sections and a `Constraint` for the native library platforms
- [x] 2.4 Add `components#vertep-rules` with `provides::`, `uses::`, `realizes::` and `scheduled_in::` before code lands
- [x] 2.5 Add a View `views/rules.md` with `describes::` edges and a Mermaid diagram of one decision

## 3. Program lifecycle

- [x] 3.1 `vertep-rules` crate: store, supersede and read program nodes through `vertep-tiramemsu`
- [x] 3.2 Closure digest and engine version recorded in the `Derivation` (the decision entry field itself is M12's)
- [x] 3.3 Fuel policy and recorded fuel used
- [x] 3.4 Typed `rules-unavailable` error on unsupported builds: tested for the build without `native`; the browser build is kept clear by a dependency-graph gate in CI (`vertep-wasm` must not reach `vertep-rules` or `leanelpi`), because the wasm toolchain cannot build here (see the design note)

## 4. Decider

- [x] 4.1 Write the offer, the actor context and a `ctx deciding <case>` statement as statements in the entry graph; every rule starts from the context statement
- [x] 4.2 `RuleDecider` returning event and support, rationale kind `derivation` (`DeciderKind::Rules` added to `vertep-agent`; support is returned through `RuleDecider::derivations`)
- [x] 4.3 Refusal when no rule applies or the event is not offered
- [x] 4.4 Conformance vectors for accept, refuse, no rule, fuel exhausted, engine version mismatch

## 5. Records

- [x] 5.1 Bundle includes the program closure, the supporting statements and the engine version; `vt rules verify-bundle` re-runs the rule offline (a `vt verify-bundle` for M12's whole bundle stays with M12)
- [x] 5.2 Counterfactual replay varies the rule version; the report names where decisions diverge
- [x] 5.3 `vt rules put|show|check|diff`
- [x] 5.4 Example of the credit policy as rules: `crates/vertep-rules/examples/credit.rs`. Left to the site and example owners: `examples/credit-decision` and the site pages that say rules are not built, which are uncommitted in another session

## 6. CI

- [x] 6.1 A `rules` job: the build without the engine, the dependency-graph gates, then the vectors against a checksum-pinned `libleanelpi` for linux-x64 (written, not yet run on GitHub)
