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

## Open Questions

- Does the LF layer produce a proof term for a program that uses `tm.*` host builtins, or must their answers be taken as a tape, as the datamodel is for `step`?
- Is totality required for a chart to declare a rules decider in a production tier?
