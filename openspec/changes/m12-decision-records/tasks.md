## 1. Graph first

- [x] 1.1 Add milestone `M12 Decision records` to `tg.md/roadmap.md`, `precedes::` from M3 Agents and M7 Durable objects
- [x] 1.2 Add Capabilities `Decision records` and `Journal redaction` to `tg.md/capabilities.md` and list both in its `openspec:` frontmatter
- [x] 1.3 Record D1-D6 as `Decision` sections and a `Constraint` `Erasure rules` with `constrains::` edges
- [x] 1.4 Add a View `views/decision-records.md` with `describes::` edges and a Mermaid diagram of bundle verification

## 2. Entry format

- [x] 2.1 Add `disposition` and `offered` to decision entries, versioned, with old entries read as `unknown` (`JournalEntry.disposition` and `.offered`, optional fields with serde defaults; set by `AgentHost::turn` from `AgentMachine::disposition_of` before the append; the same fields flow through the memory and store journals)
- [x] 2.2 Record `refused-not-offered`, `refused-guard`, `fallback-on-deadline` and `fallback-on-refusal` where the shell decides them (also `refused-invalid`, `stale` and `refused-by-decider`; `refused-guard` and `fallback-on-refusal` are defined but no chart path produces them today: a lost guard re-issues the offer so the late answer is `stale`, and no path consults a fallback after a refusal)
- [ ] 2.3 Conformance vectors for each disposition, replayed through the Lean oracle in strict mode

## 3. Explanation

- [x] 3.1 `explain(actor, position)` in `vertep-agent`, derived by replay (`AgentHost::explain` returning `explain::Explanation`: state before and after, entered and exited states, effects, and for a decision the offer, the choice, the disposition, the decider, the transitions taken and the claim; `to_json` for tools)
- [x] 3.2 Label rationale as a claim in the view and the JSON schema (the claim is its own record with `claimed_by` and the label `unverified claim by the decider`; there is no JSON schema of the view yet)
- [x] 3.2a Carry the rule decider's `Derivation` on the decision entry (as `Annotations.derivation`, with no rationale text) and show it in the explanation view as a derivation, not a claim; the offer the explanation reports is the offer the rule read, both coming from the same request (`rule-decider#Offer And State Are Statements#One record`, carried from `m14-rules-decider`)
- [ ] 3.3 Expose through `vertep-wasm` and `vertep-py`
- [x] 3.4 Update `examples/credit-decision` to print the disposition and the offer for each decision, and re-record its output (it also exports and verifies the bundle of the referred loan)

## 4. Bundles

- [x] 4.1 Bundle format: manifest, journal slice, chart versions by hash, digests (`vertep_agent::bundle::Bundle`; the chart travels as canonical N-Quads and is checked against its version hash)
- [x] 4.2 `vt explain BUNDLE` and `vt verify-bundle BUNDLE`, verifying offline (the bundle is exported by `AgentHost::export_bundle`; the CLI reads a file)
- [x] 4.3 Tamper vectors: altered entry, missing chart, wrong digest

- [x] 4.4 Retract the derived facts of `vertep-rules` that rest on an erased subject's sealed statements, and journal the retraction with the erasure (`journal-redaction#Erasure Retracts Derived Facts`, carried from `m15-rule-invariants` 3.3)

## 5. Integrity

- [x] 5.1 Opt-in hash chain and signatures on agent journals, sharing the `vertep-objects` envelope (`StoreAgentJournal::with_sealer` with `EntrySealer`, implemented by `ChainedAgentSealer` in `vertep-objects`; `verify_agent_journal` and `verify_chain` check it)
- [x] 5.2 Promote a plain actor to a durable object without rewriting history (shown for an opted-in journal: the object's writer continues the same chain in a later epoch and `verify_chain` accepts old and new entries; a journal that never opted in has no chain to promote)

## 6. Counterfactual replay

- [x] 6.1 Replay inputs under another chart version, report divergences, never write to the original journal (`AgentHost::counterfactual` with `DecisionSource::Recorded` or `Supplied`; the report says which)
- [x] 6.2 Example `fork_under_new_policy` with recorded output (as a section of `examples/credit-decision`: the approval ceiling cut to 15 000, and loan 1001's approval would go to the officer)

## 7. Redaction

- [x] 7.1 `sensitive` marker in the type language, with SHACL and JSON Schema projections (`@sensitive`, `@subject`, `@retain(P30D)`; `SealPolicy::from_chart`)
- [x] 7.2 Sealing of sensitive fields with a per-subject key behind a `KeyStore` trait
- [x] 7.3 Erasure entry and key destruction
- [x] 7.4 Replay in recorded mode after erasure, with the result marked (entries record their outcome; replay follows it after an erased value)
- [x] 7.5 Retention policy and expiry as erasure
- [x] 7.6 Decide the snapshot policy (open question in D6) and record it

## 8. Annotations and docs

- [x] 8.1 For each requirement, annotate the implementing code with `@tg: implements:: [[openspec:<capability>#<Requirement>]]`
- [x] 8.2 For each scenario, annotate its test with `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]`
- [x] 8.3 Update `site/regulated.html`, `site/articles/explainable-by-construction.html` and `site/articles/regulated-decisions.html` where they say a feature is not built
- [x] 8.4 Run `tg check`, `tg ontology check` and `tg trace decision-records --gaps`, and the same for `journal-redaction`, and close every gap
