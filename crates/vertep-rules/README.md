# vertep-rules

Rule programs beside the journal. A rules decider evaluates an ELPI program stored in the
shard's tiramemsu file over the offer and the actor's context, and returns the chosen event with
the statements its conclusion rests on as its rationale: a derivation, not a claim.

Planned in `tg.md/components.md#vertep-rules`; specified by the OpenSpec change
`m14-rules-decider`. The engine needs the native library, so everything is behind the `native`
feature (off by default); without it constructors return `RulesError::Unavailable`.

```text
LEANELPI_DIST=/path/to/lean-elpi/dist DYLD_LIBRARY_PATH=$LEANELPI_DIST \
    cargo test -p vertep-rules --features native
```
