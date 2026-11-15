## 1. Graph first

- [x] 1.1 Add milestone `M15 Rule invariants` to `tg.md/roadmap.md`, `precedes::` from M14
- [x] 1.2 Add Capabilities `Rule invariants` and `Derived facts` and list both in the `openspec:` frontmatter of `tg.md/capabilities.md`
- [x] 1.3 Record R7-R10 as `Decision` sections; extend `constraints#Erasure rules` with the derived-facts rule
- [x] 1.4 Extend `views/rules.md` with the refusal path diagram (the diagram was drafted with the view)

## 2. Enforcement

- [x] 2.1 Wire tiramemsu's commit hook through `vertep-tiramemsu`: `AppendError::Refused` in `vertep-store`, mapped from tiramemsu's `ConstraintViolated` (no feature needed: a store without enforced programs never produces it)
- [x] 2.2 Runtime refusal path: the transaction aborts, the message is dead-lettered with the violation names and statement ids and acknowledged, the actor steps down unchanged (`vertep-runtime` `node.rs`)
- [x] 2.3 Full check on enabling, registration refused when live data violates
- [x] 2.4 Property test: delta check against full check on random journals

## 3. Derived facts

- [x] 3.1 Materialize into the actor's graph with support (derived facts are members of the graph `<subject>/derived`; not the state graph itself, because the runtime compares the state graph with the machine's state after every turn and would read a derived fact as a state change)
- [x] 3.2 Retraction and reconciliation when support changes; non-monotonic label surfaced
- [x] 3.3 Erasure retracts dependents and journals the retraction (carried to `m12-decision-records` task 4.4 as the requirement `journal-redaction#Erasure Retracts Derived Facts`: it needs the key destruction that M12 builds)

## 4. Surface

- [x] 4.1 `vt rules enforce|unenforce|materialize` (and `check`, from M14)
- [x] 4.2 Conformance vectors for each requirement
- [x] 4.3 An invariant example and the docs page (`crates/vertep-rules/examples/invariant.rs` and `site/docs/rules.html`; `examples/credit-decision` itself is unchanged) (an example of an invariant is in `crates/vertep-rules/tests/invariants.rs`; the site and example belong to another session)
