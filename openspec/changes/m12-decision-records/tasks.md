## 1. Graph first

- [ ] 1.1 Add milestone `M12 Decision records` to `tg.md/roadmap.md`, `precedes::` from M3 Agents and M7 Durable objects
- [ ] 1.2 Add Capabilities `Decision records` and `Journal redaction` to `tg.md/capabilities.md` and list both in its `openspec:` frontmatter
- [ ] 1.3 Record D1-D6 as `Decision` sections and a `Constraint` `Erasure rules` with `constrains::` edges
- [ ] 1.4 Add a View `views/decision-records.md` with `describes::` edges and a Mermaid diagram of bundle verification

## 2. Entry format

- [ ] 2.1 Add `disposition` and `offered` to decision entries, versioned, with old entries read as `unknown`
- [ ] 2.2 Record `refused-not-offered`, `refused-guard`, `fallback-on-deadline` and `fallback-on-refusal` where the shell decides them
- [ ] 2.3 Conformance vectors for each disposition, replayed through the Lean oracle in strict mode

## 3. Explanation

- [ ] 3.1 `explain(actor, position)` in `vertep-agent`, derived by replay
- [ ] 3.2 Label rationale as a claim in the view and the JSON schema
- [ ] 3.2a Carry the rule decider's `Derivation` (support statements, closure digest, engine) on the decision entry with rationale kind `derivation`, and report the offer in the explanation view from the same statements the rule read (`rule-decider#Offer And State Are Statements#One record`, carried from `m14-rules-decider`)
- [ ] 3.3 Expose through `vertep-wasm` and `vertep-py`
- [ ] 3.4 Update `examples/credit-decision` to print the disposition and the offer for each decision, and re-record its output

## 4. Bundles

- [ ] 4.1 Bundle format: manifest, journal slice, chart versions by hash, digests
- [ ] 4.2 `vt explain` and `vt verify-bundle`, verifying offline
- [ ] 4.3 Tamper vectors: altered entry, missing chart, wrong digest

- [ ] 4.4 Retract the derived facts of `vertep-rules` that rest on an erased subject's sealed statements, and journal the retraction with the erasure (`journal-redaction#Erasure Retracts Derived Facts`, carried from `m15-rule-invariants` 3.3)

## 5. Integrity

- [ ] 5.1 Opt-in hash chain and signatures on agent journals, sharing the `vertep-objects` envelope
- [ ] 5.2 Promote a plain actor to a durable object without rewriting history

## 6. Counterfactual replay

- [ ] 6.1 Replay inputs under another chart version, report divergences, never write to the original journal
- [ ] 6.2 Example `fork_under_new_policy` with recorded output

## 7. Redaction

- [ ] 7.1 `sensitive` marker in the type language, with SHACL and JSON Schema projections
- [ ] 7.2 Sealing of sensitive fields with a per-subject key behind a `KeyStore` trait
- [ ] 7.3 Erasure entry and key destruction
- [ ] 7.4 Replay in recorded mode after erasure, with the result marked
- [ ] 7.5 Retention policy and expiry as erasure
- [ ] 7.6 Decide the snapshot policy (open question in D6) and record it

## 8. Annotations and docs

- [ ] 8.1 For each requirement, annotate the implementing code with `@tg: implements:: [[openspec:<capability>#<Requirement>]]`
- [ ] 8.2 For each scenario, annotate its test with `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]`
- [ ] 8.3 Update `site/regulated.html`, `site/articles/explainable-by-construction.html` and `site/articles/regulated-decisions.html` where they say a feature is not built
- [ ] 8.4 Run `tg check`, `tg ontology check` and `tg trace decision-records --gaps`, and the same for `journal-redaction`, and close every gap
