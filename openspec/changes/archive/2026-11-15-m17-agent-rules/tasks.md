## 1. Graph first

- [x] 1.1 Add milestone `M17 Agent rules` to `tg.md/roadmap.md`, `precedes::` from M15 and M16
- [x] 1.2 Add Capability `Rule authoring` to `tg.md/capabilities.md` and its frontmatter
- [x] 1.3 Record R15-R18 as `Decision` sections and a `Constraint` for self-governance
- [x] 1.4 Extend `views/agents.md` (Self-modification) and `views/rules.md`

## 2. Gate

- [x] 2.1 Draft program state, authorship on the node
- [x] 2.2 Ordered gate with structured error terms
- [x] 2.3 Trial run on recorded cases under fuel; waiver path
- [x] 2.4 Conformance vectors for each gate step

## 3. Shadow and promotion

- [x] 3.1 Shadow evaluation with a sampling rate (`shadow_every`) and a difference record
- [x] 3.2 Staged promotion as journaled acts under the promote grant
- [x] 3.3 Self-governance check against program scope and actor address
- [x] 3.4 Rule diff with the decisions that would differ

## 4. Surface

- [x] 4.1 Agent tool to propose rules under a grant (`vt:tool/rules/propose`; the identity is the tool's, never the caller's argument)
- [x] 4.2 `vt rules propose|shadow|promote|diff` (`propose`, `shadow`, `promote`; `diff` is the line diff of M14 and `authoring::diff` gives the clause diff)
- [x] 4.3 Example: an agent repairs a rejected proposal from the error term (as a test: `a_type_error_names_the_clause_and_a_repair_passes`)
- [x] 4.4 Docs page (`site/docs/rules.html`)
