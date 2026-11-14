## Context

The journal records inputs: start, events, decisions, model results, timers. A decision entry today carries the chosen event, the correlation id of the offer and the decider's annotations. The offer itself and the outcome of validation are not on the entry, so a reader must recompute the offer from the chart and the state, and must infer a refusal from the next state. `examples/credit-decision` and the article *Explainable by construction* make the consequence concrete.

Plain actor journals are append-only by compare-and-set; durable objects add a hash chain and signatures. Entries hold payloads in the clear.

## Goals / Non-Goals

**Goals:**

- A journal alone, with its chart versions, answers: what state, what was offered, who chose, under which rules, what followed, and was the answer accepted.
- A third party can verify an exported record offline.
- Personal data can be erased from a journal without invalidating it, and replay says honestly what it can and cannot reproduce afterwards.
- No change to the semantics of the step function: everything here is in the shell, the entry format and tooling.

**Non-Goals:**

- Explaining the model's weights, or scoring the faithfulness of a rationale.
- Legal advice or a compliance mapping that claims more than the record contributes.
- A key-management service; a trait with an in-memory and a file implementation is enough.

## Decisions

### D1. Disposition and offer are fields of the decision entry

A decision entry gains `disposition` and `offered` (the candidate events enabled at the offer, in offer order). Alternatives rejected: a separate "refusal" entry (splits one fact in two and breaks entry-per-input), and recomputing the offer in the explanation view (needs the chart and the datamodel as of the entry, which is exactly what makes the journal alone insufficient for a quick read).

Reasons: [[decisions#Event-sourced actors]] makes the journal the truth; an offer is an input the shell produced, so recording it is recording an input.

### D2. The explanation view is derived, never stored

`explain(actor, position)` is computed from entries and chart versions by replay, so it cannot disagree with the journal. The bundle stores the journal slice, not a precomputed explanation, and `verify-bundle` recomputes the view and compares it with the manifest.

### D3. Rationale is typed as a claim

Annotations from a decider are carried in the view as `claimed_by` plus text, under a heading that the schema and the docs name as unverified. Alternative rejected: merging the rationale into a single "reason" field, which invites readers to treat it as established.

### D4. Agent journal integrity reuses the durable-objects chain

Opt-in, per actor type, using the same entry hash and signature envelope as `vertep-objects`. Alternative rejected: a second format. Reasons: one verifier, one set of test vectors, and a plain actor can be promoted to a durable object without rewriting history.

### D5. Erasure is key destruction over sealed fields

Sensitive fields are sealed with a per-subject data key (AEAD); the entry hash covers the ciphertext. Erasure deletes the key and appends an `erasure` entry naming the subject and time. The chain stays valid. Alternatives rejected: rewriting entries (breaks the chain and every digest that cites it), and keeping personal data out of events by convention only (what users must do today; brittle and unchecked).

### D6. Replay after erasure uses recorded transitions and says so

Entries already record the transitions they took. When a sealed field a guard reads is unreadable, replay does not re-evaluate that guard; it follows the recorded transition and marks the result `replay: recorded` instead of `replay: verified`. Open question for implementation: whether a snapshot taken before erasure may still carry the erased value, and the policy for purging snapshots (proposed: snapshots hold sealed fields too, under the same key).

## Risks / Trade-offs

- **Entry size and write cost grow** with the offer and with sealing. Mitigation: the offer is a list of short event names; benchmarks in `m13-production-readiness` track the cost.
- **Recorded replay weakens the verification claim after erasure.** That is intended, and the view must never present it as full verification.
- **Format change.** Additive and versioned; old journals report `disposition: unknown` rather than fail.
- **Key custody is the new weak point.** Destroying a key must be real; the trait documents what a conforming key store guarantees.

## Migration Plan

1. Add fields and versioning behind a feature, read old entries as `unknown`.
2. Explanation view, then bundle and CLI.
3. Integrity opt-in, then sealing and erasure, then retention.
4. Update the site pages that say "not built".

## Open Questions

- Should the offer record candidate payload schemas as well as names?
- Who may request an erasure, and how is the request authenticated? (A federation grant is the likely answer.)
- Is a per-subject key the right granularity for cases with several data subjects?
