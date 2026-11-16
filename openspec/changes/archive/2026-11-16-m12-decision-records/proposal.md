## Why

Vertep's strongest claim to regulated users is that a decision's record is the process itself, replayable without the model. Building the credit-decision example (`examples/credit-decision`) showed where that claim is still thin:

1. **A refused answer reads like an accepted one.** When the underwriter answers `approve` for a loan that was not offered `approve`, the journal prints `3 decision approve`, and only the state as of entry 3 (`officer_review`) shows that nothing was approved. An auditor must infer the refusal. The record should say it.
2. **The offer is not part of the explanation.** What was offered is visible to a live decider and in a transcript, but a reader of the journal alone has to recompute it. The explanation should carry it.
3. **There is no exportable artifact.** A regulator or a counterparty cannot be handed "this decision, verifiably" without handing over a runtime.
4. **Plain agent journals are not tamper-evident.** Durable objects have hash-chained, signed journals; a plain actor's journal is append-only by contract only.
5. **Append-only and erasure collide.** GDPR Article 17 and similar rules require removing personal data. A journal that cannot be redacted forces users to keep personal data out of events, which is brittle. There is no specified way to erase and still replay.
6. **Counterfactuals are possible but not specified.** `fork_from_journal` exists as an example; "what would this case have done under the next policy version" is not a requirement with scenarios.

## What Changes

- **New capability `decision-records`:**
  - Journal entries for decisions record a **disposition** (`accepted`, `refused-not-offered`, `refused-guard`, `fallback-on-deadline`, `fallback-on-refusal`) and the **offer** (the enabled candidate events) as of the decision.
  - An **explanation view**: for any actor and position, a structured record of the state, the offer, the chosen event, the decider's identity and rationale (labelled as the decider's claim), the chart version, the transitions taken and the effects.
  - An **explanation bundle**: a self-contained, content-addressed export (journal slice, chart versions by hash, manifest with digests) that verifies offline with no decider and no network, with `vt explain` and `vt verify-bundle`.
  - **Integrity for agent journals:** opt-in hash chaining and signatures on agent journals, reusing the durable-objects journal format.
  - **Counterfactual replay:** replay a journal's inputs under another chart version, report where states diverge, and leave the original untouched.
- **New capability `journal-redaction`:**
  - Chart types mark fields **sensitive**; sensitive payload fields are sealed in the entry under a per-subject data key.
  - **Erasure by key destruction:** destroying a subject's key makes their fields unreadable; the journal stays valid, replay continues in recorded-transition mode and says so, and the erasure is itself journaled.
  - **Retention policy** per chart type, with expiry as erasure.
- **Docs:** `site/articles/explainable-by-construction.html` and `site/regulated.html` already state these as not built; they are updated when each requirement lands.

Out of scope: opening the model (attribution scores), legal classification of any system, key-management services beyond a pluggable trait, and a hosted audit portal.

## Capabilities

### New Capabilities

- `decision-records`: dispositions, offers, explanations, verifiable bundles, journal integrity and counterfactual replay.
- `journal-redaction`: sensitive fields, sealing, erasure by key destruction and retention.

### Modified Capabilities

None. `deciders` and `event-journal` gain fields additively; see Impact.

## Impact

- `vertep-agent` (decision entries, offer capture, explanation view), `vertep-runtime` and `vertep-store` (entry format additions, sealing hook), `vertep-objects` (journal chain format reused), `vertep-cli` (`vt explain`, `vt verify-bundle`), `vertep-conformance` (bundle and redaction vectors), `vertep-wasm` and `vertep-py` (explanation view through the bridge).
- Journal entry format changes are additive and versioned; an old journal opens in a new build and reports its entries as `disposition: unknown`.
- tg.md sections touched: `roadmap#M12 Decision records` (new), `capabilities#Decision records` (new), `capabilities#Journal redaction` (new), `views/agents#Journaling and replay`, `decisions` (D1-D6), `constraints#Erasure rules`.
- Decisions that bind the area: [[decisions#Event-sourced actors]], [[decisions#Full SCXML first]].
- Adds requirements `[[openspec:decision-records#Decision Disposition Is Recorded]]`, `[[openspec:decision-records#Offer Is Part Of The Record]]`, `[[openspec:decision-records#Explanation View]]`, `[[openspec:decision-records#Rationale Is Labelled As A Claim]]`, `[[openspec:decision-records#Explanation Bundle]]`, `[[openspec:decision-records#Agent Journal Integrity]]`, `[[openspec:decision-records#Counterfactual Replay]]`, `[[openspec:journal-redaction#Sensitive Fields]]`, `[[openspec:journal-redaction#Erasure By Key Destruction]]`, `[[openspec:journal-redaction#Replay After Erasure]]` and `[[openspec:journal-redaction#Retention Policy]]`.
