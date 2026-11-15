## Why

Vertep already lets an LLM write a plan chart and run it sandboxed (`plan-charts`, the CEL-only policy). Rules extend the same idea to policy: an agent that sees a pattern in refused cases may propose a new rule. The danger is symmetrical to its value: a rule is a decider, so an agent that can write its own governing rules can remove its own limits.

tiramemsu already rejects a program that live data violates at registration, and elpilite showed a useful gate: capabilities, types, safety, a trial run, and errors as structured terms an agent can repair. This change defines authorship, gating, shadowing and promotion so that agent-proposed rules are reviewable and never self-governing.

## What Changes

- **New capability `rule-authoring`:**
  - A proposed rule passes a **gate** before it can be stored live: capability check for the proposing identity, type check against the chart's event types, profile check (M16), a trial run on recorded cases under fuel, and, for invariants, a check against existing data.
  - Gate failures are **structured errors** (a stable code and the offending clause) so an agent can repair and resubmit.
  - **Authorship is recorded** on the program node: proposer identity, grant, source of the proposal.
  - **Shadow evaluation**: a proposed program runs beside the live one on real offers and records where it differs, without deciding.
  - **Staged promotion**: shadow, then live for a named scope, then live; each step an explicit, journaled act by an identity with the promote grant.
  - **No self-governance**: an actor cannot propose, edit or promote a rule program that governs its own decisions or invariants.
  - **Rule diff**: a readable comparison of two versions with the decisions that would differ.

Out of scope: learning rules automatically from data, a UI for rule review, and cross-shard policy distribution.

## Capabilities

### New Capabilities

- `rule-authoring`: gate, structured errors, authorship, shadow evaluation, staged promotion, self-governance limit and diff.

### Modified Capabilities

None. It uses grants from `did-identity` and the plan-chart policy of `plan-charts` as they are.

## Impact

- `vertep-rules` (gate, shadow, promotion), `vertep-agent` (a tool for proposing rules under a grant), `vertep-federation` (the grants), `vertep-cli` (`vt rules propose|shadow|promote|diff`), `vertep-conformance`.
- Depends on M14, M15, M16 and on the capability grants of M5.
- tg.md sections touched: `roadmap#M17 Agent rules` (new), `capabilities#Rule authoring` (new), `views/agents.md` (Self-modification), `views/rules.md`, `decisions` (R15-R18), `constraints` (self-governance).
- Decisions that bind the area: [[decisions#Event-sourced actors]], [[decisions#Explanation is the process record, not a story about it]].
- Adds requirements `[[openspec:rule-authoring#Proposals Pass A Gate]]`, `[[openspec:rule-authoring#Gate Errors Are Structured]]`, `[[openspec:rule-authoring#Authorship Is Recorded]]`, `[[openspec:rule-authoring#Shadow Evaluation]]`, `[[openspec:rule-authoring#Promotion Is Staged And Journaled]]`, `[[openspec:rule-authoring#No Self-Governance]]` and `[[openspec:rule-authoring#Rule Diff]]`.
