## Why

M14 lets a rule choose among offered events. Some policy is not a choice but a limit: no approval without a prior officer review, no payment above a ceiling without two approvals, no write to a closed case. Today such limits live in guards inside the chart, where a chart author can forget them, or in application code the journal never sees.

tiramemsu can enforce an ELPI program at commit: `tm.violation-delta`, seeded by the transaction's added and retracted statements, refuses a transaction that violates it. It can also materialize conclusions whose support is the statements they follow from, and retract them when the support is retracted. This change brings both to Vertep journals and settles how they meet erasure.

## What Changes

- **New capability `rule-invariants`:** an enforced rule program refuses a journal append that violates it; the refusal is recorded without writing the entry; enabling enforcement checks existing data; the delta check and the full check agree.
- **New capability `derived-facts`:** a rule program may materialize facts with stated support; they appear in the actor's graph with their cause, retract when their support retracts, and follow the redaction rules of `journal-redaction`.
- Runtime behaviour on refusal: the sender is told, the message is dead-lettered with the violation name, and the actor's state does not change.

Out of scope: invariants across shards (one writer per file), verification of the programs (`m16-verified-rules`), agent-written invariants (`m17-agent-rules`).

## Capabilities

### New Capabilities

- `rule-invariants`: commit-time refusal, the refusal path, enabling and the delta-versus-full equivalence.
- `derived-facts`: materialized conclusions with support, retraction and erasure.

### Modified Capabilities

None. `event-journal` gains a refusal outcome for an append; see Impact.

## Impact

- `vertep-rules` (enforcement, materialization), `vertep-tiramemsu` (commit hook wiring), `vertep-runtime` (refusal path, dead letter), `vertep-conformance` (invariant vectors), `vertep-cli` (`vt rules enforce`).
- Depends on M14 and on M12's `journal-redaction`.
- tg.md sections touched: `roadmap#M15 Rule invariants` (new), `capabilities#Rule invariants` and `capabilities#Derived facts` (new), `views/rules.md`, `decisions` (R7-R10), `constraints#Erasure rules`.
- Decisions that bind the area: [[decisions#Event-sourced actors]], [[constraints#Tiramemsu has a single writer per file]].
- Adds requirements `[[openspec:rule-invariants#Enforced Program Refuses A Violating Append]]`, `[[openspec:rule-invariants#Refusal Is Visible And Writes No Entry]]`, `[[openspec:rule-invariants#Enabling Checks Existing Data]]`, `[[openspec:rule-invariants#Delta Check Agrees With Full Check]]`, `[[openspec:derived-facts#Derived Facts Carry Support]]`, `[[openspec:derived-facts#Derived Facts Retract With Support]]` and `[[openspec:derived-facts#Erasure Retracts Dependents]]`.
