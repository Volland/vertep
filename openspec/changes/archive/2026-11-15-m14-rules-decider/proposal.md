## Why

Vertep's claim to regulated users is that a decision's record is the process itself (see [[decisions#Explanation is the process record, not a story about it]]). A decider's rationale is still the decider's claim, and the only deterministic decider, the guard decider, picks the first enabled candidate in document order. Policy that matters (credit rules, prior-authorization criteria) is written as guards inside the chart or hidden in a model.

tiramemsu 0.12 already runs ELPI (λProlog, by lean-elpi) over a view, stores programs as versioned nodes, and lets a conclusion state the statements it rests on. The journal and the rules can live in one file. This change makes rules a first-class, versioned, replayable decider whose rationale is a derivation over the recorded facts, not a story.

## What Changes

- **New capability `rule-programs`:** rule programs are nodes in the shard's tiramemsu file; a decision entry pins the program closure by digest, the engine version and the fuel; evaluation is fuel-bounded and deterministic; platforms without the native library answer a typed `unsupported` error.
- **New capability `rule-decider`:** `RuleDecider`, a fifth decider kind. The offer and the actor's state are written as statements in the entry's graph, the program reads them, and the decision carries the support statements as its rationale, labelled a derivation and not a claim. An offer with no applicable rule is a refusal. Explanation bundles include the program closure. Counterfactual replay can vary the rule version.
- **New crate `vertep-rules`** holding the decider, the program lifecycle and the pinning; `vertep-tiramemsu` gains a `rules` feature that enables tiramemsu's `elpi` and `native` features.
- **First task group is a throwaway spike** on `examples/credit-decision`, run before the rest, with an explicit go/no-go.

Out of scope: commit-time invariants and derived facts (`m15-rule-invariants`), verification of derivations (`m16-verified-rules`), agent-written rules (`m17-agent-rules`), a rules runtime for WebAssembly or Windows, and a shared policy file across shards.

## Capabilities

### New Capabilities

- `rule-programs`: storage, versioning, pinning, fuel and platform behaviour of rule programs.
- `rule-decider`: the decider, derivation rationale, offer as statements, bundle inclusion and rule-version counterfactuals.

### Modified Capabilities

None. `deciders` gains a decider kind additively; `decision-records` (M12) must accept a derivation rationale, see Impact.

## Impact

- New crate `vertep-rules`; `vertep-tiramemsu` (feature `rules`), `vertep-agent` (decider kind `Rules`, rationale kind `derivation`), `vertep-cli` (`vt rules`), `vertep-conformance` (rule vectors), `examples/credit-decision`.
- Depends on M12 (`decision-records`: dispositions, offer, bundle, counterfactual replay) and on M13 (production readiness) having been archived.
- tg.md sections touched: `roadmap#M14 Rules decider` (new), `capabilities#Rule programs` and `capabilities#Rule decider` (new), `components#vertep-rules` (new), `views/rules.md` (new), `decisions` (R1-R5), `constraints` (native library platforms).
- Decisions that bind the area: [[decisions#Two storage traits with tiramemsu as default]], [[decisions#Event-sourced actors]], [[decisions#Explanation is the process record, not a story about it]].
- Adds requirements `[[openspec:rule-programs#Rule Programs Live In The Shard]]`, `[[openspec:rule-programs#Rule Version Is Pinned By The Entry]]`, `[[openspec:rule-programs#Evaluation Is Bounded]]`, `[[openspec:rule-programs#Native Library Is Optional]]`, `[[openspec:rule-decider#Rule Decider]]`, `[[openspec:rule-decider#Derivation Is The Rationale]]`, `[[openspec:rule-decider#Offer And State Are Statements]]`, `[[openspec:rule-decider#No Applicable Rule Is A Refusal]]`, `[[openspec:rule-decider#Bundle Includes Rule Closure]]` and `[[openspec:rule-decider#Rule Version Counterfactual]]`.
