## Context

`vt certify` writes a chart's N-Quads to a file and runs the Lean checker as a process ([[decisions#vt certify runs the Lean checker as a process]]); the Lean `step` is a differential oracle for the Rust core. lean-elpi (Lean 4.35.0, same as `vertep-lean`) has an LF layer (`Elpi/LF`: signatures, recorded proof terms, export to Lean inductives, kernel-checked certificates) and a totality and mode analysis. tiramemsu keeps a Datalog oracle for Horn programs (E12). Support in tiramemsu is stated by the program and checked only against the eids the run read.

## Goals / Non-Goals

**Goals:** make "derivation" checkable by a third party; mark honestly what is and is not checked.

**Non-Goals:** verifying the ELPI engine, covering all of λProlog, or judging a policy.

## Decisions

### R11. A rules profile defines what can be checked

The profile is typed, function-free in rule heads except the declared constructors, stratified in negation, with no `findall`, cut, I/O or time builtins, and all builtins from `tm.*`. Programs outside it run (M14) but their decisions are marked `unchecked` in the view. Alternative rejected: checking all of ELPI (undecidable in general, and the engine passes a part of the upstream tests).

### R12. Totality is a static check over offers

For each decide state, the check enumerates the chart's candidate events and requires, for every combination of offer subsets, a solution or an explicit `refuse` clause, within the profile's decidable fragment. A program that cannot be shown total is reported with a counterexample offer. Alternative rejected: a runtime-only "no rule applies" (M14 R4 stays; this check turns it from a surprise into a design-time finding).

### R13. Support is checked for necessity, and certificates check sufficiency

Offline, removing each supporting statement and re-running must fail to conclude (necessity). The LF certificate shows that the conclusion follows from the support under the program (sufficiency). The two together replace "the author said so". Alternative rejected: trusting the stated support after the eid check.

### R14. Certificates are produced by the engine and checked by Lean as a process

Same split as `vt certify`: a Rust-side step exports the derivation (program closure, support statements, proof term) to a file; `vertep-lean` checks it. The pinned axioms test extends to the new theorems. Alternative rejected: linking the checker (the process split already exists and keeps the Lean runtime out of the Rust build).

## Risks / Trade-offs

- **lean-elpi's LF layer maturity** is unverified here; the first task is a go/no-go on one real derivation.
- **Totality can be expensive** for large offers; mitigation: bound it by the chart's candidate count and report `unknown` rather than guess.
- **Two lean-elpi consumers** (the engine through tiramemsu, the certificate checker in Lean) must pin the same version; the journal's recorded engine version is checked against the certificate's.

## Migration Plan

1. Profile and the checker, with `unchecked` marking.
2. Differential oracle vectors.
3. Support necessity in `verify-bundle`.
4. Totality.
5. Certificates and the Lean checker, with axioms pinned.

## Go/no-go result (2026-11-15): partial

A delegated, read-only investigation on a copy of lean-elpi v0.4.1 (Lean 4.35.0) found:

- **Horn derivations over a fact tape: go.** `lf_signature`, `lf_export` and `lf_certify` turned a small signature (facts standing in for `tm.triple` answers, one rule) and the query `holds c1 approve` into a theorem the Lean kernel accepts with no axioms; a query with no derivation is refused; a corrupted certificate is rejected by lean-elpi's own test.
- **Live host builtins: no-go.** The LF layer is a closed language with no hook for host predicates, so the answers to `tm.triple` must be a tape: an ordered list of ground `(S, P, O, eid)` tuples, one per answer consumed, produced by the Rust engine beside the derivation and trusted as input. The kernel then proves that the conclusion follows from this support under the program (R13's sufficiency); that each statement is really in the store stays with the existing eid check.
- **Untested:** negation, lists, `findall` and cut (expected unsupported), scale at thousands of facts, and whether emitted rules match the ELPI program (needs a differential test).
- **Smallest path (recommended): skip lean-elpi's search.** Rust emits Lean source (inductives for the terms, the facts and the rules as constructors, `theorem cert := .r_decide .t2 .t1`) and bare `lean` checks it in about half a second with no new `require` in the lakefile; the encoding mirrors `lf_export`. The alternative is to depend on lean-elpi from `vertep-lean`, which needs a pinned version and a tiramemsu-to-LF translation.
- **Blocker for either path:** tiramemsu returns the support but not the derivation tree (which clause, which facts, in what order), so a Rust exporter needs a replay or recording step. Estimates, not measured: exporter 5 to 8 days, Lean checker 2 to 4 days, pinned-axiom and corrupted-certificate tests 1 to 2 days.

Decision: certificates (group 5) stay open and are not started here; the profile, support necessity, totality and the Datalog oracle are built and tested.

## Built (2026-11-15)

`check_profile` (lexical, with a stratification check on predicate names), `Derivation.profile_checked`, `RuleBundle::unnecessary_support`, `totality` (every non-empty subset of up to 8 candidates under the given sample contexts, with `unknown` beyond that; a program refuses on purpose by concluding `refuse`), an ELPI-against-Datalog vector on a random graph, and `vt rules profile|unnecessary`. The totality answer holds for the sample contexts only; it does not reason about all values.

## Open Questions

- Does the LF layer produce a proof term for a program that uses `tm.*` host builtins, or must their answers be taken as a tape, as the datamodel is for `step`?
- Is totality required for a chart to declare a rules decider in a production tier?
