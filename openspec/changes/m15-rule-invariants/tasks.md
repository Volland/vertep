## 1. Graph first

- [ ] 1.1 Add milestone `M15 Rule invariants` to `tg.md/roadmap.md`, `precedes::` from M14
- [ ] 1.2 Add Capabilities `Rule invariants` and `Derived facts` and list both in the `openspec:` frontmatter of `tg.md/capabilities.md`
- [ ] 1.3 Record R7-R10 as `Decision` sections; extend `constraints#Erasure rules` with the derived-facts rule
- [ ] 1.4 Extend `views/rules.md` with the refusal path diagram

## 2. Enforcement

- [ ] 2.1 Wire tiramemsu's commit hook through `vertep-tiramemsu` behind `rules`
- [ ] 2.2 Runtime refusal path: abort, dead-letter with violation name, message and eids
- [ ] 2.3 Full check on enabling, registration refused when live data violates
- [ ] 2.4 Property test: delta check against full check on random journals

## 3. Derived facts

- [ ] 3.1 Materialize into the actor's graph with support
- [ ] 3.2 Retraction and reconciliation when support changes; non-monotonic label surfaced
- [ ] 3.3 Erasure retracts dependents and journals the retraction (needs `journal-redaction`)

## 4. Surface

- [ ] 4.1 `vt rules enforce|unenforce|check`
- [ ] 4.2 Conformance vectors for each requirement
- [ ] 4.3 Update `examples/credit-decision` with one invariant and the docs page
