## Why

After M14 and M15 a decision's rationale is a derivation, but the host only checks that every supporting statement was read by the run. It does not check that the conclusion really follows from them, that the program covers every offer, or that the engine agrees with another engine. For regulated users "derivation" must mean something a third party can check, and Vertep's differentiator is that it already checks its own core with Lean (`vt certify`, the differential oracle).

lean-elpi has a native LF layer that records proof terms and gives kernel-checked certificates, and tiramemsu already compares ELPI with a Datalog engine on Horn programs. This change uses both.

## What Changes

- **New capability `rule-verification`:**
  - A **totality check**: for a rules profile, every offer of a decide state yields a decision or an explicit refusal, never a silent gap.
  - A **support-soundness check** run offline over a bundle: re-running the rule with its support removed fails to conclude, so the stated support is necessary for the conclusion.
  - A **differential oracle**: Horn rule programs give the same conclusions in the ELPI engine and in the Datalog engine already in tiramemsu.
  - A **derivation certificate**: a decision's derivation is exported as an LF proof term and checked by `vt certify` in Lean, offline.
- A documented **rules profile**: the subset of the language for which these checks are defined; programs outside it still run but are marked unchecked.

Out of scope: proving the engine itself correct, a Rust evaluator for the profile (a possible later milestone), and any claim about the fairness or legality of a policy.

## Capabilities

### New Capabilities

- `rule-verification`: totality, support soundness, the differential oracle and derivation certificates.

### Modified Capabilities

None. `formal-model` and `explanation bundles` gain checks additively.

## Impact

- `vertep-rules` (profile checker, support-soundness check), `vertep-lean` (certificate checker for derivations), `vertep-cli` (`vt rules verify`, `vt certify` for rules), `vertep-conformance` (oracle vectors).
- Depends on M14, on lean-elpi's LF layer at the version the engine pins, and on the Lean toolchain (v4.35.0, shared with `vertep-lean`).
- tg.md sections touched: `roadmap#M16 Verified rules` (new), `capabilities#Rule verification` (new), `views/verification.md`, `views/rules.md`, `decisions` (R11-R14).
- Decisions that bind the area: [[decisions#Lean verification bridge]], [[decisions#vt certify runs the Lean checker as a process]].
- Adds requirements `[[openspec:rule-verification#Rules Profile]]`, `[[openspec:rule-verification#Totality Check]]`, `[[openspec:rule-verification#Support Is Necessary]]`, `[[openspec:rule-verification#Engines Agree On Horn Programs]]` and `[[openspec:rule-verification#Derivation Certificate]]`.
