## 1. Go/no-go

- [ ] 1.1 Export one real derivation from the credit-decision policy as an LF proof term and check it in Lean; decide whether host builtins need an answer tape
- [ ] 1.2 Record the result in `design.md`

## 2. Graph first

- [ ] 2.1 Add milestone `M16 Verified rules` to `tg.md/roadmap.md`, `precedes::` from M14
- [ ] 2.2 Add Capability `Rule verification` to `tg.md/capabilities.md` and its frontmatter
- [ ] 2.3 Record R11-R14 as `Decision` sections
- [ ] 2.4 Extend `views/verification.md` and `views/rules.md`

## 3. Profile and oracle

- [ ] 3.1 Profile checker, `unchecked` marking in the explanation view
- [ ] 3.2 Horn program vectors: ELPI engine against the Datalog engine in tiramemsu
- [ ] 3.3 Conformance vectors for in-profile and out-of-profile programs

## 4. Checks

- [ ] 4.1 Support necessity in `vt verify-bundle`
- [ ] 4.2 Totality check with counterexample offers
- [ ] 4.3 `vt rules verify`

## 5. Certificates

- [ ] 5.1 Export format for a derivation and the Lean checker in `vertep-lean`
- [ ] 5.2 `vt certify` accepts a bundle and checks every rule decision in it
- [ ] 5.3 Pin the axioms of the new theorems in `VertepTests.Axioms`
- [ ] 5.4 Update the verification docs page and the regulated-decisions article
