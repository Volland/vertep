## 1. Go/no-go

- [x] 1.1 Export one real derivation from the credit-decision policy as an LF proof term and check it in Lean; decide whether host builtins need an answer tape (result: PARTIAL, see `design.md`; a delegated investigation with scratch evidence, not re-run by the maintainer)
- [x] 1.2 Record the result in `design.md`

## 2. Graph first

- [x] 2.1 Add milestone `M16 Verified rules` to `tg.md/roadmap.md`, `precedes::` from M14
- [x] 2.2 Add Capability `Rule verification` to `tg.md/capabilities.md` and its frontmatter
- [x] 2.3 Record R11-R14 as `Decision` sections
- [ ] 2.4 Extend `views/verification.md` and `views/rules.md` (`views/rules.md` is extended; `views/verification.md` is not yet)

## 3. Profile and oracle

- [x] 3.1 Profile checker, `unchecked` marking in the explanation view (`Derivation.profile_checked`; the explanation view that shows it is M12)
- [x] 3.2 Horn program vectors: ELPI engine against the Datalog engine in tiramemsu
- [x] 3.3 Conformance vectors for in-profile and out-of-profile programs

## 4. Checks

- [x] 4.1 Support necessity in `vt verify-bundle` (as `vt rules unnecessary BUNDLE` and `RuleBundle::unnecessary_support`; the whole-bundle `vt verify-bundle` is M12)
- [x] 4.2 Totality check with counterexample offers (`vertep_rules::totality`, sampled over the contexts given; no CLI command yet)
- [x] 4.3 `vt rules verify` (as `vt rules profile` and `vt rules unnecessary`)

## 5. Certificates

- [ ] 5.1 Export format for a derivation and the Lean checker in `vertep-lean` (open: see the go/no-go; the exporter needs the derivation tree, which tiramemsu does not give)
- [ ] 5.2 `vt certify` accepts a bundle and checks every rule decision in it
- [ ] 5.3 Pin the axioms of the new theorems in `VertepTests.Axioms`
- [ ] 5.4 Update the verification docs page and the regulated-decisions article
