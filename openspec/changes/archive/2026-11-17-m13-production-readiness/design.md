## Context

The review's evidence: 26 workspace crates and about 190k lines of Rust in four days; 405 requirements with 403 implemented and 401 verified; one external star and no issues. What is established is internal consistency. What an adopter needs next is external evidence and a smaller surface to trust.

## Goals / Non-Goals

**Goals:**

- Every claim an adopter relies on has a number, a test or a stated tier behind it.
- A new contributor builds every artifact from a clean checkout using only documented steps.
- Each known open item is a requirement with scenarios, so `tg trace --gaps` tracks it.

**Non-Goals:**

- New features. This change adds measurement, packaging, protocol completion and honesty about maturity.
- Benchmarks against other products. A comparison page can cite the suite later; the suite measures Vertep.
- A hosted service.

## Decisions

### D1. Tiers are data in the manifests

Each crate and package declares `stable`, `preview` or `experimental` in `[package.metadata.vertep]` (npm: `vertep.tier` in `package.json`). The README and site tables are generated from it by a script that CI checks, so a table cannot drift. Alternative rejected: a prose table, which is how the drift began.

Initial assignment, to be argued in review and not taken as final: `stable` for the core, surfaces, datamodels, runtime and conformance (they pass external suites); `preview` for agents, wasm and the bindings; `experimental` for cluster, federation, objects, webvh, wallet, kolyada and mediator until each has a user outside the project.

### D2. The toolchain is a script and a container, not a note

A pinned `scripts/build-wasm.sh` and a container image definition encode the LLVM and `wasm-bindgen` versions and the workaround that is now known only to the maintainer. CI builds `@vertep/web` and the Python wheel in that container from a clean checkout. Alternatives rejected: documenting the workaround (decays), vendoring LLVM (heavy).

Reasons: bus factor. A project whose browser build runs on one laptop is a one-laptop project.

### D3. Benchmarks measure the claims

Moved 2026-11-17: the benchmark suite, its gate and its results page are carried by `m18-runtime-performance` (phase 0), so that M13 could be archived without them.


Suite: pure `step` throughput on three chart sizes; journal append latency and replay speed per backend (memory, tiramemsu file); decider round trip with a scripted decider; `@vertep/web` and wheel size; cold start in the browser. The gate fails CI on a regression over a stated threshold against the last release's recorded results. Results are published on a page with the machine and commit recorded.

### D4. Journal shipping is a pull protocol over the cluster transport

A follower pulls entries after its last position, receives them in order with their digests, and acknowledges. Pull gives natural back-pressure and resumption. Alternatives rejected: push with windowing (more state on the leader), and shipping snapshots only (loses the entry-level audit trail).

### D5. WebSocket is a transport of relay mailboxes, not a second protocol

Live delivery reuses the mailbox envelope and acknowledgement of the existing relay; the socket only removes polling latency and must fall back to polling.

### D6. Scope freeze

After M10 and M11 are finished, no new milestone starts until this change is archived. Reasons: adoption comes from depth in the strongest layers, not from a thirteenth feature.

## Risks / Trade-offs

- **Benchmarks on shared CI are noisy.** Mitigation: a dedicated runner or a relative threshold with repeated runs, and results recorded with machine details.
- **Tier labels can embarrass.** They are meant to; an honest `experimental` is worth more than an unqualified "built".
- **Container builds are slow.** Cache layers by version pins; a nightly job covers the full build.

## Open Questions

- Which backend is the default durable store for agents once the doc audit settles whether the durable agent journal is in?
- Is there a runner with stable performance available for the gate?
