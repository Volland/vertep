## Why

Twelve milestones were built in four days. The specification is complete for what exists, and what is missing is the evidence and the edges that an adopter checks before relying on it. An independent review (2026-11-14) ranked the project strong on specification discipline, core semantics and formal verification, and weak on adoption readiness and scope control. This change closes the technical items behind that second list so that "0.1.x" means "young" and not "unknown":

1. **No published performance numbers.** Claims such as "survives the loss of any one organisation" and "replays without a model" have no throughput, latency or size figures behind them, and no regression guard.
2. **The WebAssembly build depends on one machine.** Homebrew LLVM broke after a `z3` upgrade and the `@vertep/web` build works through a DYLD wrapper known to the maintainer. A contributor cannot be expected to rediscover it.
3. **Open items are scattered.** Journal shipping between cluster nodes has no wire protocol (M4). Live delivery over WebSocket is not bound (M5). The Lean proof of custody safety under custodian-set changes is open (M7 task 10.4). They are real, but they sit in prose inside roadmap sections.
4. **Maturity is not declared.** The README and site mix crates that have run under conformance suites with crates that have run only their own tests. A reader cannot tell which is which.
5. **Docs and behaviour can disagree.** The decider-agent example page warns that only the in-memory agent journal exists, while the M7 roadmap section says the durable agent journal was delivered. The code has `StoreAgentJournal` (durable-objects, Durable Agent Journal), so the page was stale; it was corrected on 2026-11-14 and the audit requirement below keeps it from recurring.
6. **No supply-chain posture.** No dependency audit gate, no SBOM, no signed release artifacts, for packages that are meant to sit near regulated decisions.
7. **No tested path from install to first success.** The ten-minute path in the site copy is not a CI check.

## What Changes

- **New capability `production-readiness`:** stability tiers per crate and package; a reproducible toolchain with a CI job that builds `@vertep/web` and the Python wheel from a clean checkout; a supply-chain gate (advisories, licences, SBOM, signed releases); a docs-versus-behaviour audit; and a tested ten-minute path.
- **New capability `journal-shipping`:** a wire protocol for journal replication between cluster nodes, with ordering, back-pressure and resumption, using the existing journal format and the transport of `vertep-cluster`.
- **Modified capability `relay-mailboxes`:** live delivery over WebSocket.
- **Tracked, not re-specified here:** the Lean proof of custody safety under custodian-set changes stays in `m7-durable-objects` task 10.4 and is listed in this change's tasks as a dependency to close; the M10 open items stay in `m10-agent-protocols`.
- **Process:** after M11 and M10 are finished, a scope freeze for new milestones until this change is archived. The freeze is recorded as a decision, not a requirement.

## Capabilities

### New Capabilities

- `production-readiness`: tiers, toolchain, supply chain, doc audit, first-run path. The benchmark suite moved to `m18-runtime-performance` (phase 0) on 2026-11-17, before this change was archived.
- `journal-shipping`: replicating a journal between nodes.

### Modified Capabilities

- `relay-mailboxes`: WebSocket live delivery.

## Impact

- New `benches/` and a results page under `site/`; new CI jobs and a `deny.toml`; `Cargo.toml` metadata for tiers; `bindings/js` and `bindings/python` build scripts; `vertep-cluster` (shipping), `vertep-federation` (WebSocket).
- tg.md sections touched: `roadmap#M13 Production readiness` (new), `capabilities#Production readiness`, `capabilities#Journal shipping`, `capabilities#Relay mailboxes`, `components#vertep-cluster`, `components#vertep-federation`, `views/distribution`, `constraints#Toolchain pins`, `decisions` (D1-D5).
- Decisions that bind the area: [[decisions#Event-sourced actors]].
- Adds requirements `[[openspec:production-readiness#Stability Tiers]]`, `[[openspec:production-readiness#Reproducible Toolchain]]`, `[[openspec:production-readiness#Supply Chain Gate]]`, `[[openspec:production-readiness#Docs Match Behaviour]]`, `[[openspec:production-readiness#Ten Minute Path]]`, `[[openspec:journal-shipping#Ordered Shipping]]`, `[[openspec:journal-shipping#Resumption]]`, `[[openspec:journal-shipping#Back Pressure]]` and `[[openspec:relay-mailboxes#WebSocket Live Delivery]]`.
