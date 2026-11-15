# vertep-rules

Rule programs beside the journal. A rules decider evaluates an ELPI program stored in the
shard's tiramemsu file over the offer and the actor's context, and returns the chosen event with
the statements its conclusion rests on as its rationale: a derivation, not a claim.

Planned in `tg.md/components.md#vertep-rules`; specified by the OpenSpec change
`m14-rules-decider`. The engine needs the native library, so everything is behind the `native`
feature (off by default); without it constructors return `RulesError::Unavailable`.

What is here: `put_program` (store or supersede a program in the shard's file), `RuleDecider`
(a `Decider`; every rule starts from a `ctx deciding` statement), `Derivation` (the support of a
decision, the closure digest, the engine and the fuel), `verify` (replay as of the decision's
transaction), `RuleBundle` (a decision that re-runs offline), `counterfactual` (another program
version tried on recorded decisions) and `check_program`. `vt rules` in `vertep-cli` (feature
`rules`) wraps them. `cargo run -p vertep-rules --features native --example credit` shows the flow.

```text
LEANELPI_DIST=/path/to/lean-elpi/dist DYLD_LIBRARY_PATH=$LEANELPI_DIST \
    cargo test -p vertep-rules --features native
```
