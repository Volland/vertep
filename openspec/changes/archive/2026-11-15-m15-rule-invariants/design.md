## Context

A journal append is one `Db::transact` (see `views/persistence`). tiramemsu runs enforced programs in the commit hook with `tm.violation-delta Name Msg Eids` seeded by the transaction's delta, and runs the full `tm.violation` when enforcement is turned on or the program text changes. Materialization (`tm.conclude`) writes derived statements whose support is a list of eids plus the program's source; editing a library retracts what depended on it.

## Goals / Non-Goals

**Goals:**

- A limit stated once as a rule refuses violating appends for every actor in the shard, whoever wrote the chart.
- A refusal is observable and leaves no trace in the actor's state or entry sequence.
- Derived facts are explainable and never outlive their cause.

**Non-Goals:** cross-shard invariants, liveness properties, and anything that needs a read outside the transaction's view.

## Decisions

### R7. A refused append writes no entry and dead-letters the message

If the commit hook refuses, the transaction aborts as for a head conflict, nothing is written, and the runtime moves the message to the dead-letter set with the violation name, message and offending eids. Alternative rejected: writing a refusal entry (it would be an input the actor never processed and would complicate replay: M12 already has `refused-*` dispositions for decisions, and an append refusal is a different thing, the shell's, not the decider's).

### R8. The delta check is an optimization, the full check is the meaning

Each enforced program states both forms where it can. A property test runs random transactions and requires that an enforced program refuses exactly the transactions whose full check gains a violation (tiramemsu E12). Alternative rejected: delta-only programs with no full form, because enabling enforcement on live data then cannot be checked.

### R9. Derived facts live in the actor's graph with their cause

Materialized facts are written into the graph of the actor they are about, not into a separate store, so `View::dependents` and the explanation view reach them. Reconciliation reruns when support changes. Programs using negation, cut or `findall` are labelled non-monotonic by the host and listed for rerun after writes; that label is shown, not hidden.

### R10. Erasure retracts what depends on the erased

When a subject's key is destroyed (M12), statements derived from their sealed fields are retracted, and the retraction is itself journaled with the erasure entry. A derived fact that depended only on non-erased statements survives. Alternative rejected: keeping derived facts that quote erased values (defeats erasure).

## Risks / Trade-offs

- **Commit latency.** An enforced program runs in the single writer's critical section; a slow program lowers shard throughput. Mitigation: a fuel budget per commit and M13's benchmark gate.
- **Non-monotonic programs** make reconciliation costlier and the label conservative. Mitigation: a wrong label costs a rerun, never a wrong store (tiramemsu E4).
- **Redaction depends on M12 landing first**, and its snapshot policy is an open question there.

## Migration Plan

1. Enforcement hook and the refusal path, behind the `rules` feature.
2. Enable-time full check, then the delta property test.
3. Materialization into the actor graph, then the erasure rule.
4. `vt rules enforce`, docs.

## Built (2026-11-15)

Enforcement, the refusal path, the full check on enabling, the delta-versus-full property test, materialization with support and retraction with support, and `vt rules enforce|unenforce|materialize` exist in `vertep-rules`, `vertep-store`, `vertep-tiramemsu`, `vertep-runtime` and `vertep-cli`. The refused turn is checked through a running node: nothing is written, the dead letter names the constraint, and another actor is unaffected.

Findings: (1) the enforcement hook is tiramemsu's, so it applies to every commit in the file, including a host's own writes, not only journal appends; a rule that mentions a status set by the host refuses the host's write too (the property test relies on this); (2) a program needs both a `tm.violation-delta` form for the commit and a `tm.violation` form for enabling, and the two must say the same thing, which the property test checks on 60 random operations; (3) derived facts are written to the shard, not yet to a named graph per actor; (4) erasure retraction (R10) waits on M12.

## Open Questions

- Should a refusal also be reported to the actor's supervisor as an event?
- Is a per-shard invariant enough for regulated cases that span actors in several shards?
