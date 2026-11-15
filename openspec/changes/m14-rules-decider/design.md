## Context

tiramemsu 0.12 (feature `elpi`, off by default; `native` links `libleanelpi`) provides: programs as nodes with one `elpi:source` statement, edited by supersede; `accumulate <iri>.` includes; host builtins `tm.triple S P O E` and `tm.valid`; `tm.conclude Fact Support` with a check that every supported eid was returned by a builtin; fuel; `View::elpi`, `View::elpi_program_info`. `vertep-tiramemsu` writes one journal append as one `Db::transact`; entries are named graphs, state is derived with the entry as support, and `as_of` gives time travel. The workspace pins tiramemsu 0.12.0 with `default-features = false, features = ["sparql"]`.

`libleanelpi` carries the Lean runtime and exists for darwin and linux on arm64 and x64; on wasm and Windows tiramemsu's ELPI calls answer `Unsupported`.

## Goals / Non-Goals

**Goals:**

- A decider whose answer is a derivation over recorded facts, replayable with no model and no network.
- Rules versioned with the journal: any decision can be re-run against the rules in force when it was taken, or against a later version.
- No change to the step function; everything is in the shell and the entry format.

**Non-Goals:**

- Proving derivations correct (M16), enforcing invariants (M15), agent-written rules (M17).
- Running rules in the browser or on Windows.
- Opening ELPI's full language to untrusted authors; M14 documents a rules profile but enforces it only by fuel and the host builtins.

## Decisions

### R1. Rules are per shard, in the journal's file

A rule program is a node in the same tiramemsu file as the journal it decides for. Alternatives rejected: one shared policy file (needs replication and version pinning across shards, and tiramemsu has one writer per file); rules inside the chart (couples policy releases to chart versions and hides them from the graph). Reason: one transaction covers the entry and the rules it was decided under, and `as_of` already gives versioned rules. A policy shared by many shards is installed by writing the same program into each; the program digest shows they agree.

### R2. The rationale is the support of the conclusion

`RuleDecider` runs a goal that must bind a `tm.conclude (decision Event Payload) Support`. The decision entry stores the support eids, the program closure digest and the engine version. The view labels it `derivation`, distinct from a decider's `claim`. Alternative rejected: storing the proof tree (the host cannot see one; ELPI's backtracking makes "the facts used" something the author threads, see tiramemsu E3). Reason: this is what the host can check, that the eids were returned by the run, and it is enough to re-run offline.

### R3. The offer and the state are statements

Before the rule runs, the shell writes the offered candidates and the actor's context as statements in the entry's graph, so `tm.triple` can read them. This is also how M12's "the offer is part of the record" is satisfied, with no second copy. Alternative rejected: passing the offer as a goal argument (the support could not point at it).

### R4. No applicable rule is a refusal, with a reason

If the goal has no solution or concludes an event not in the offer, the decider returns `Refusal`. The chart's existing `error.decide.invalid` path and the M12 disposition (`refused-not-offered`, `fallback-on-refusal`) apply unchanged. There is no silent default; a chart that wants one declares a fallback decider.

### R5. Evaluation is fuel-bounded and pinned

Every run has a step budget set by policy, never by wall clock. The entry records engine version, fuel limit and fuel used. Replay with a different engine version warns and, in strict mode, fails. Alternative rejected: pinning only the program (an engine upgrade could change a conclusion invisibly).

### R6. The native library is optional behind a `rules` feature

`vertep-rules` and `vertep-tiramemsu/rules` are off by default. Without them, loading a chart that declares a `Rules` decider fails with a typed `rules-unavailable` error, never a silent fallback to another decider. The WebAssembly and Python-on-unsupported-platform builds say so in their docs.

## Risks / Trade-offs

- **Platform reach.** No browser, no Windows. Mitigation: the error is typed and documented; a Rust evaluator for a rules profile is the later option, deliberately not decided here.
- **Engine maturity.** lean-elpi is a young engine (about 2.4 times slower than ELPI, partial upstream-test coverage). Mitigation: fuel, version pinning, and the differential oracle in M16.
- **Stated, not proven, support.** An author can state a support that is not the real reason. The host checks the eids were returned by the run, not that the conclusion depends on them. M16 narrows this.
- **Redaction.** A derivation over statements that are later erased cannot be re-run. M15 defines the interaction; until then a rule must not read a sealed field.
- **Journal entry size** grows with the support list. Benchmarks from M13 apply.

## Migration Plan

1. Spike (throwaway): enable the feature, one program on `examples/credit-decision`, go/no-go on whether the refusal case reads clearly.
2. `vertep-rules` crate, program lifecycle, pinning.
3. `RuleDecider`, offer and state as statements, refusals.
4. Bundle inclusion and rule-version counterfactuals, then the CLI.
5. Update `site/regulated.html` and the credit-decision article where they say rules are not built.

## Spike result (2026-11-15): go

Task group 1 ran as a throwaway crate against tiramemsu 0.12.0 with feature `elpi` and `libleanelpi` from the lean-elpi 0.4 dist, on darwin-arm64. It is not in the repository.

- **Builds and runs** with `LEANELPI_DIST` set and the dylib on `DYLD_LIBRARY_PATH`; no code in Vertep changed.
- **Derivation as rationale works.** The credit-decision policy as one program node concluded `refer` for the 80 000 case with support `[offered refer, amount 80000]`, and `approve` for a 20 000 case with support `[offered approve, dti, amount]`.
- **Rule versions replay.** After a supersede that lowered the ceiling to 30 000, a 40 000 case concluded `refer` now and `approve` at the earlier transaction, and `program_info` named a different source statement at each.
- **Fuel works.** A looping program ended with a typed out-of-fuel error at 10 000 steps.
- **Cost.** One conclusion took 0.3 to 1.8 ms on a small file. With 2 000 cases in the shard, an unscoped `elpi_conclude` took 118 ms (release) because it enumerates every case, so a decision would cost the size of the shard. Scoping the rule with a context statement (`ctx vt:deciding <case>` as the first goal, so the rule starts from one subject) took 0.27 ms with the same 2 000 cases, and its support includes the context statement.
- **Library size.** `libleanelpi.dylib` is 154 MB. That is a real cost for wheels and release binaries and belongs in M13's tiers and size budget.
- **Forged support** is not shown by this spike: the engine's type checker rejects a literal eid (`int` against `stmt`), so a forge would have to reuse an eid from another `tm.triple`, which tiramemsu's own suite tests.
- **Not done:** wiring the program into a `Decider` (task 1.2 is half done: the policy and the run are shown, the trait is not), and the refused-answer path through the chart.

Consequences for the design: R3 gains a context statement naming the case being decided and every rule starts from it; the support of a decision always includes it; and `vertep-rules` must not call the unscoped form.

## Open Questions

- Should the program closure digest be the hash of the source statements or of the normalized clauses?
- Does `vt rules` write programs through the runtime or directly to the file when the shard has a live writer?
- Which ELPI builtins stay out of the profile (`std.findall`, IO, time)?
