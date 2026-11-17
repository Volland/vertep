## Why

Rule decisions run only where the native engine library `libleanelpi` exists: macOS and Linux on two architectures, about 154 MB with the Lean runtime. The browser build, Windows and the Python wheels refuse a chart with a rules decider (`rules-unavailable`), and a bundle made on a server cannot be checked in a browser. Most decision policies sit inside the rules profile M16 defined, and the Horn part of that profile is plain Datalog with stated support. Tiramemsu already ships a pure-Rust Datalog engine, `tm-datalog`, which the M16 oracle tests against ELPI.

## What Changes

- A **portable engine** in `vertep-rules`: a pure-Rust evaluator for the portable profile (Horn clauses over `tm.triple` and `tm.conclude`, comparisons and arithmetic on ground numbers and strings, no `findall`, no negation, no effects). It runs on every target, including `wasm32-unknown-unknown`, Windows and the Python wheels, and links no native library.
- Engine **selection**: the native engine when it is built in, else the portable engine for a program inside the portable profile. A program outside the portable profile on a build without the native engine still fails with `rules-unavailable`, naming the first clause that needs it. Nothing falls back to a non-rules decider.
- **Agreement**: on programs inside the portable profile both engines give the same conclusions and the same stated support. The M16 differential oracle grows into a conformance suite over the decision vectors and pseudo-random graphs, run in CI.
- **Records**: the derivation records which engine ran (`portable/<version>` or `lean-elpi/<version>`). Replay uses the recorded engine family. A bundle whose program is inside the portable profile can be verified by either engine, and the report says `cross-engine` when it was.
- **Bindings**: the browser build (`@vertep/web`) and the Python package gain the rules decider, `verifyBundle` and counterfactual replay for rule decisions, on the portable engine.
- The stated-support check (`forged-support`) and the fuel budget hold in the portable engine. Fuel is counted in the engine's own steps and is compared only within one engine.

Out of scope: negation and `findall` in the portable profile, totality and certificates on the portable engine beyond what M16 already defines, and removing the native engine.

## Capabilities

### New Capabilities

- `rule-engines`: the portable engine, the portable profile, engine selection, agreement between engines, and cross-engine bundle verification.

### Modified Capabilities

- `rule-programs`: `Native Library Is Optional` now refuses only programs outside the portable profile.
- `rule-decider`: bundles name their engine, and verification may cross engines for portable programs.

## Impact

- `crates/vertep-rules` (new `portable` module, engine selection; the `native` feature stays and is still off by default), `crates/vertep-wasm`, `crates/vertep-py`, `bindings/python`, `bindings/js/packages/web` types.
- CI: the conformance suite runs on Linux with the native library, and the portable engine runs on the wasm and Windows jobs.
- Docs: `site/docs/rules*.html`, `site/docs/browser.html`, `site/docs/python.html`, and the tiers in `site/docs/tiers.html`.
- Depends on `rule-verification` (M16) for the profile, and on `tm-datalog` 0.12 from tiramemsu.
