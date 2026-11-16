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

The offer is recorded in a form from which statements in the entry's graph can be written, so that a rule decider (`m14-rules-decider`) reads the same record the explanation view reports and no second copy exists.

### D2. The explanation view is derived, never stored

`explain(actor, position)` is computed from entries and chart versions by replay, so it cannot disagree with the journal. The bundle stores the journal slice, not a precomputed explanation, and `verify-bundle` recomputes the view and compares it with the manifest.

### D3. Rationale is typed as a claim

Annotations from a decider are carried in the view as `claimed_by` plus text, under a heading that the schema and the docs name as unverified. A rationale has a kind, `claim` or `derivation`; a derivation (from a rule decider, `m14-rules-decider`) lists the supporting statements and the program closure and is not labelled unverified. The bundle manifest has room for rule program closures and an engine version. Alternative rejected: merging the rationale into a single "reason" field, which invites readers to treat it as established.

### D4. Agent journal integrity reuses the durable-objects chain

Opt-in, per actor type, using the same entry hash and signature envelope as `vertep-objects`. Alternative rejected: a second format. Reasons: one verifier, one set of test vectors, and a plain actor can be promoted to a durable object without rewriting history.

### D5. Erasure is key destruction over sealed fields

Sensitive fields are sealed with a per-subject data key (AEAD); the entry hash covers the ciphertext. Erasure deletes the key and appends an `erasure` entry naming the subject and time. The chain stays valid. Alternatives rejected: rewriting entries (breaks the chain and every digest that cites it), and keeping personal data out of events by convention only (what users must do today; brittle and unchecked).

### D6. Replay after erasure uses recorded transitions and says so

Entries already record the transitions they took. When a sealed field a guard reads is unreadable, replay does not re-evaluate that guard; it follows the recorded transition and marks the result `replay: recorded` instead of `replay: verified`. Open question for implementation: whether a snapshot taken before erasure may still carry the erased value, and the policy for purging snapshots (proposed: snapshots hold sealed fields too, under the same key).

### Built (2026-11-16): sealing, erasure and retention

`SealingJournal` in `vertep-agent::redaction` wraps any `AgentJournal`, below the chained sealer, so the hash and signature cover ciphertext. A `SealPolicy` names, per event, a JSON pointer to the field and one to the subject; the field becomes `{"$sealed": {subject, nonce, ct, until?}}` under ChaCha20-Poly1305 with the actor, position and pointer as associated data, so a ciphertext cannot be moved to another entry. Reading opens what the `KeyStore` can open and shows `{"$erased": subject}` otherwise. `erase` destroys the key first, then appends an `erasure` entry (a new `AgentInput`, ignored by the machine) to each journal that held the subject; repeating it is a no-op, and a destroyed subject can never get a new key. `expire(now)` erases subjects whose sealed value has a passed `until`. Open: following recorded transitions (D6 assumed entries record them; they record inputs, so replay re-evaluates a guard that reads an erased value and may diverge), the type-language marker, derived-fact retraction and the snapshot policy.

### Built (2026-11-15): the entry format

`JournalEntry` gained optional `disposition` and `offered`. They are not part of `AgentInput`, so replay and the machine ignore them: the machine recomputes every check itself, and the entry fields are the shell's record of what it saw. The host asks `AgentMachine::disposition_of(&input)` before it appends, which repeats the checks of `on_decision` (annotations, candidate, payload schema, enablement) without changing anything. Findings: (1) when a guard turns false while an offer is open, the candidate set shrinks and the offer is re-issued under a new correlation, so the late answer is `stale`, not `refused-guard`; (2) the chart never consults a fallback after a refusal, so `fallback-on-refusal` exists in the enum and is not produced; (3) the Lean oracle reads inputs only, so strict replay of dispositions through Lean (task 2.3) is not done.

### Built (2026-11-15): the explanation view

`AgentHost::explain` replays the journal into a fresh machine up to the entry before and up to the entry, so the view is a difference of two replays, never stored. Findings: (1) it reports entered and exited states and the effects of the entry, not the individual transitions of every microstep: only a decision exposes the transitions it took (`AcceptedDecision.taken`), so for other inputs the state difference stands in; (2) a cold host with no decider bound gives the same explanation as the host that ran the decision, and altering the journal changes it (both tested); (3) for an entry that predates dispositions the view says `unknown` and `offered_known: false` instead of recomputing, which keeps D2 (derived, but never a guess about what the shell saw).

### Built (2026-11-16): the bundle

A `Bundle` holds the journal entries 1 to `to`, each chart version the journal uses as canonical N-Quads keyed by its version IRI, and a manifest of SHA-256 digests (per entry, per chart, per explained position). `verify` re-hashes each chart's text and checks it against the manifest and against the version it names, checks each entry's digest, replays the entries into a fresh host with no decider, recomputes the explanation of every position in the range and compares its digest. Errors name the entry or the chart. Findings: (1) the chart travels as RDF N-Quads and is rebuilt by `read_chart`, so its identity is the same hash the runtime already uses; (2) the manifest is not signed, so a party who rewrites entries and manifest together makes a consistent bundle: the integrity requirement (hash-chained, signed agent journals) is what binds a bundle to a signer, and is not built; (3) a decision made by the rule decider carries its derivation in `Annotations.derivation` and has no rationale text, so the explanation shows a derivation and no claim.

### Built (2026-11-16): integrity and counterfactual replay

**Integrity.** The agent crate cannot depend on `vertep-objects`, so it defines the hook: `StoreAgentJournal::with_sealer` takes an `EntrySealer` that may add statements to the store entry just before it is appended, given the stored previous entry. `vertep-objects` implements it as `ChainedAgentSealer`: the same custody link, content hash and signature as a durable object's entries, with the actor's address as the object, epoch 1 unless set, and the signer's DID as the writer. `verify_agent_journal` is `verify_chain` over the store. A plain `StoreAgentJournal::new` journal is unchanged. Findings: (1) an unsealed journal fails verification as `malformed` at position 1, which is the honest answer; (2) the hash covers the entry as stored, including the JSON of the input, so altering an input is a hash mismatch at that position, and swapping two entries is caught at the first one; (3) an external event's message id embeds the process id, so two runs of the same scenario produce different bytes and different hashes: the chain is over what happened, not over what would be reproduced; (4) the in-memory `MemoryJournal` has no store entries to seal and stays unchained.

**Counterfactual replay.** `AgentHost::counterfactual` runs the recorded chart and the chart being tried side by side into scratch machines, applies each recorded input to both, and records both configurations and what the shell would have made of each answer under the new chart. With `Supplied`, a function answers the offers the new chart makes in place of the recorded decisions. The journal is only read. Finding: a stricter chart usually changes the *handling* before it changes the outcome: in the example, cutting the ceiling makes the recorded approve of loan 1001 refused, and the chart's own invalid-decision path sends the case to the officer.

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
