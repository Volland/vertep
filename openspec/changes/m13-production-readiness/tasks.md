## 1. Graph first

- [x] 1.1 Add milestone `M13 Production readiness` to `tg.md/roadmap.md`, `precedes::` from M11, with a note that new milestones wait for it
- [x] 1.2 Add Capabilities `Production readiness` and `Journal shipping` to `tg.md/capabilities.md` and list both in its `openspec:` frontmatter
- [x] 1.3 Record D1-D6 as `Decision` sections and `Toolchain pins` as a `Constraint`

## 2. Tiers and docs honesty

- [x] 2.1 Tier metadata in every crate and npm package manifest
- [x] 2.2 Script that renders the README and site tier tables, and a CI check that they are current
- [x] 2.3 Audit every "not yet" and "delivered" statement in `site/` and `tg.md/roadmap.md` against the code, starting with the agent-journal warning on the decider-agent page
- [x] 2.4 Fix each disagreement, in the code or the text

## 3. Toolchain

- [x] 3.1 `scripts/build-wasm.sh` with pinned versions and the LLVM workaround
- [x] 3.2 Container definition and a CI job that builds `@vertep/web` and the Python wheel from a clean checkout
- [x] 3.3 Document the one-command build in `CONTRIBUTING.md`

## 4. Benchmarks

- [ ] 4.1 (owned by `m18-runtime-performance` phase 0) `benches/` for step, journal append, replay, decider round trip
- [ ] 4.2 Size and cold-start measurements for `@vertep/web` and the wheel
- [ ] 4.3 Regression gate against the last release's recorded results
- [ ] 4.4 Results page under `site/` with machine and commit

## 5. Supply chain

- [x] 5.1 `deny.toml` for advisories and licences, enforced in CI
- [x] 5.2 SBOM per release (`scripts/sbom.sh`, `release.yml`; first run at release 0.1.12)
- [x] 5.3 Signed release artifacts and provenance for crates, npm and wheels (attestations for npm tarballs, wheels and SBOMs; crates.io has no signing, see `constraints#crates.io has no signed provenance`)

## 6. Ten-minute path

- [x] 6.1 A CI job that follows the site's quick start on a clean runner and the credit-decision example, and fails if either breaks

## 7. Journal shipping and WebSocket

- [x] 7.1 Pull protocol on `vertep-cluster`: ordered entries with digests, acknowledgements
- [x] 7.2 Resumption after a dropped connection and after a follower restart
- [x] 7.3 Back-pressure under a slow follower
- [x] 7.4 WebSocket transport for relay mailboxes with polling fallback in `vertep-federation`
- [x] 7.5 Fault histories for shipping replayed through the Lean lease model in strict mode

## 8. Dependencies closed elsewhere

- [x] 8.1 (10.4 refuted as stated and corrected; its proof moved to `m19-custody-proof`, M7 archived 2026-11-17) Confirm `m7-durable-objects` task 10.4 (Lean proof of custody safety under custodian-set changes) is closed, or move it here
- [x] 8.2 Confirm the open items of `m10-agent-protocols` are closed or explicitly deferred with a ticket (all closed 2026-11-16; archived)

## 9. Annotations and checks

- [ ] 9.1 For each requirement, annotate the implementing code with `@tg: implements:: [[openspec:<capability>#<Requirement>]]`
- [ ] 9.2 For each scenario, annotate its test or CI job with `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]`
- [ ] 9.3 Run `tg check`, `tg ontology check` and `tg trace production-readiness --gaps`, and the same for `journal-shipping`, and close every gap
