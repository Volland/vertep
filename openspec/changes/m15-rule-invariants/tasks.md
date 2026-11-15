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

- [ ] 3.1 Materialize into the actor's graph with support (done: `materialize` writes derived facts with support into the shard; open: placing them in the actor's named graph)
- [x] 3.2 Retraction and reconciliation when support changes; non-monotonic label surfaced
- [ ] 3.3 Erasure retracts dependents and journals the retraction (needs `journal-redaction`)

## 4. Surface

- [x] 4.1 `vt rules enforce|unenforce|materialize` (and `check`, from M14)
- [x] 4.2 Conformance vectors for each requirement
- [ ] 4.3 Update `examples/credit-decision` with one invariant and the docs page (an example of an invariant is in `crates/vertep-rules/tests/invariants.rs`; the site and example belong to another session)
