## Context

M14 stores programs as versioned nodes; M15 enforces and materializes them; M16 defines a checkable profile. Agents act under capability grants (M5) and may run plan charts under the CEL-only policy (M3). The `views/agents` Self-modification section sets the principle that an agent changes behaviour only through reviewed, journaled steps.

## Goals / Non-Goals

**Goals:** let an agent propose rules usefully, make bad proposals fail with repairable errors, and make it impossible for an agent to change the rules that bind it.

**Non-Goals:** automatic rule induction, human review UI, shared policy across shards.

## Decisions

### R15. A proposal is a program node in a draft state

A proposed program is stored as a node with state `proposed`, never read by a decider or enforced until promoted. Alternative rejected: holding proposals outside the store (they would lose authorship and history).

### R16. The gate is ordered and its errors are terms

Order: capability, types, profile, trial run on recorded cases under fuel, existing-data check for invariants. The first failure stops with a stable error code, the clause and the reason. Alternative rejected: free-text errors, which agents cannot repair reliably.

### R17. Shadow before live, and promotion is a journaled act

A shadow run evaluates the proposal on each real offer beside the live program, records the difference, and has no effect on decisions. Promotion steps (shadow, scoped live, live) are each an entry in a rules journal by an identity holding a promote grant. Alternative rejected: a single "activate" flag (no evidence between proposal and effect).

### R18. An actor cannot govern itself

A proposal, edit or promotion is refused if the proposing identity is an actor whose decisions or appends the program would govern. The check uses the program's declared scope against the actor's address. Alternative rejected: relying on grants alone (a broad grant could still include self-governance; this is a structural limit, not a permission).

## Risks / Trade-offs

- **Trial run on recorded cases** depends on a corpus; with none, the gate reports `no-cases` and requires an explicit waiver by a promote-grant holder.
- **Scope declaration** can be wrong; mitigated by checking against the programs' actual reads of statements about actors.
- **Shadow load** doubles rule evaluation cost in the writer's path; a sampling rate is part of the shadow request.

## Migration Plan

1. Draft state, authorship and the gate with structured errors.
2. Shadow evaluation and diff.
3. Staged promotion and the self-governance check.
4. An agent tool, `vt rules propose`, docs.

## Built (2026-11-15)

`vertep_rules::authoring`: `propose` (self-governance, capability, event types, profile, trial run on recorded cases, existing-data check, in that order, each failure a `GateError` with a code), proposals stored as program nodes of their own under `<program>/proposal/<digest>` with authorship and stage statements, `shadow`, `promote` (one stage at a time, a journaled act per step, `Live` supersedes the live program), `diff` and `vt rules propose|shadow|promote`. Findings: (1) the type check scans for event IRIs in the text, so it catches a concluded event the chart does not declare but not a mistyped argument; (2) the capability check is a closure the host supplies, and the CLI reads it from a JSON grants file; wiring federation grant credentials into that closure is an integration for the host, not a requirement of this change; (3) `Scoped` records the scope, and a `RuleDecider::with_scope` runs the proposal scoped to its scope and the live program otherwise (tested; a scope is a decider setting, not a per-request value); (4) a promoter cannot be the proposer's own governed actor, but nothing stops the proposer from promoting if it also holds the grant.

## Open Questions

- Who holds the promote grant by default, and can it be a quorum?
- Is a waiver for "no cases" itself journaled in the same way as promotion?
