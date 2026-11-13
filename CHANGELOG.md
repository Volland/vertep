# Changelog

All notable changes to Vertep are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Vertep uses
[Semantic Versioning](https://semver.org/); before 1.0, a minor version may break APIs.

All crates of the workspace and both npm packages share one version.

## [0.1.4] - 2026-11-13

Layout is formatting: charts drawn in the Vertepnyk designer keep their version. Change
`layout-namespace`.

### Added

- **Layout namespace** `https://w3id.org/vertep/layout#` (prefix `vl`), exported as
  `vertep_core::ir::VL_NS` and from `vertep-scxml`.
  - **SCXML:** the reader drops every `vl` attribute and element and lists it in
    `LossReport::formatting` as `LossKind::Layout`. Strict mode accepts it, and other foreign
    namespaces are unchanged.
  - **YAML:** an optional top-level `layout` key is accepted and ignored; the writer never
    emits it.
  - Layout never reaches the IR, so moving a box never changes the chart hash or version IRI.

### Fixed

- **`@vertep/web` in Electron workers:** the client and worker no longer mistake a Web Worker
  with Node integration (Obsidian, for one) for Node's `worker_threads`. Before, the worker
  waited on `node:worker_threads` and the client never got an answer.

### Changed

- The `m8-webvh` and `layout-namespace` changes are archived; their requirements are in
  `openspec/specs/webvh-identity`, `webvh-hosting`, `scxml-format` and `yaml-format`.
- CI tests the cluster shard hash on `macos-15-intel` in place of the retired `macos-13`
  runner.

## [0.1.3] - 2026-11-13

Milestone M8: `did:webvh` hosting. Change `m8-webvh`; design in
`tg.md/views/durable-objects.md`.

### Added

- **`vertep-webvh`** (new crate): `did:webvh` authoring and hosting.
  - **Authoring:** create, update, rotate with pre-rotation, and deactivate DID logs. Every
    signature goes through the `KeyManager` (`KmSigner`); no private key reaches a log, a file
    or the caller.
  - **Documents:** built from Vertep identities, with `TSPTransport`, `DIDCommMessaging` and
    `VertepCustody` services and the object controller's key.
  - **Verification before publishing:** errors name the first failing entry, or say how many
    witness proofs are missing.
  - **Custodians as witnesses** (feature `custody`):
    - an object's log names its custodians as witnesses at the custody quorum;
    - a custodian witnesses only valid next entries, signed by a current update key, that
      declare a custodian set it accepted;
    - custody changes carry their log entry, and the writer publishes it after the switch
      (`WebvhPublisher`).
  - **Publishers:** directory, git and command, writing the `did:webvh` 1.0 layout
    (`did.jsonl`, `did-witness.json`, `did.json`).
  - **Log server** (feature `server`): content types, CORS, a cache lifetime capped by `ttl`,
    GET and HEAD only, no path traversal.
  - **Fetchers:** `DirectoryFetcher` and `RoutedFetcher` map domains to directories or local
    servers.
- **`vt webvh`:** `create`, `update`, `rotate`, `deactivate`, `verify`, `witness`, `publish`,
  `serve`, `object create` and `custody change`.
- **`vt-mediator`:** `did = "webvh:<host>[:<path>]"` creates the mediator's own `did:webvh` on
  first start, keeps it across restarts and serves its log beside `/tsp`.
- **`vertep-objects`:**
  - a custody `witness` request and `witnessed` response (also in the custody ontology);
  - an `EntryWitness` policy for replicas (`Replica::set_witness`);
  - `WitnessSource`, implemented by `CustodyHost`;
  - `CustodyHost::set_publisher`;
  - `CustodyFixture::for_object` and `rebind` in the testkit.

### Changed

- **Breaking:** `DocumentPublisher::publish` takes a `&dyn WitnessSource`, through which the
  publisher collects the custodians' witness proofs.

### Known limitations

- **Interop:** the DIF test-suite vector `witness-update` lowers its own witness threshold and is
  refused, as `didwebvh-rs` refuses it. A witness change applies only after the entry that makes
  it.
- **Not supported:** moving a DID to another domain, and watchers.

## [0.1.2] - 2026-11-13

Milestone M7: durable objects. Change `m7-durable-objects`; design in
`tg.md/views/durable-objects.md`.

### Added

- **`vertep-objects`** (new crate): durable objects.
  - **Identity:** an object is a virtual actor with a `did:webvh` address that never changes,
    whichever host serves it.
  - **Verifiable journal:** every entry states its object, position, custody epoch, writer,
    the previous entry's hash, its own hash and the writer's signature, so anyone can verify a
    replica from genesis (`chain`).
  - **Decentralised custody:** the object's DID document names its custodians. *Hosts* may
    write the object; *keepers* store encrypted copies they cannot read.
  - **One writer per epoch:** a host writes only after a majority of custodians promised it a
    new epoch (Paxos over the whole log), with no coordinator, lease authority or clock.
  - **Commit before effects:** an entry commits once a quorum accepted it, and the runtime
    performs a turn's sends and timers only after commit.
  - **Moving and changing:** handoff, failover, equivocation evidence, and controller-signed
    custodian-set changes by joint consensus.
  - **Over TSP:** custody travels as TSP messages, directly or through a store-and-forward
    mediator (feature `tsp`).
  - **Runtime integration:** `ObjectStorage` and `serve_object` run objects in
    `vertep-runtime`, which needs no changes.
  - **Example:** `three_orgs` runs a support agent across three organisations and survives
    the loss of the first.
- **`vertep-tsp-mediator`** (new crate, `vt-mediator` binary): a TSP mailbox mediator. TSP 1.0
  defines no store-and-forward; this adds it.
  - A TSP 1.0 routed-mode intermediary with durable mailboxes per recipient VID on tiramemsu.
  - A versioned pickup protocol carried inside TSP (`register`, `status`, `deliver`,
    `received`, `unregister`) and live WebSocket sessions.
  - Quotas and retention, and a replicated deployment.
  - Prometheus metrics, a Docker image and a two-mediator compose file.
  - Interop with the `tsp_sdk` reference wallet.
- **`vertep-federation`:**
  - `tsp_mailbox`: the client of a TSP mediator mailbox, which acknowledges after the journal
    append.
  - `live`: live delivery over WebSocket for TSP and for DIDComm Pickup 3.0
    (`live-delivery-change`).
- **`vertep-agent`:** `StoreAgentJournal` keeps an agent's inputs in any `JournalStore`, so
  agents survive restarts and fork from storage. An agent journaled into a custody store is a
  durable object.
- **`vertep-cluster`:** `ReplicaLink` and `ship_suffix`, the suffix shipping of standby
  replication generalised over the batch type. Custody reuses them.
- **Lean (`vertep-lean`):**
  - Custody model `Custody`: one writer per epoch and committed-log survival are proven for a
    fixed custodian set.
  - `CustodyJoint`: custodian-set changes by joint consensus. Quorum intersection and safety
    for runs without a change are proven. Safety for runs that change the set is an open
    conjecture, checked by random search and by replay.
  - `CustodyReconfigUnsafe`: counterexamples to the first reconfiguration rule.
  - `vertep-custody`: replays the Rust fault histories in strict mode.
- **Repository:**
  - `CHANGELOG.md`.
  - The durable-objects docs page and architecture view.
  - Decisions, constraints and the M7 milestone in `tg.md`.

### Changed

- `AgentJournal` gains `try_read`, with a default that calls `read`. The agent host replays
  through it, so a failing store is a `HostError::Journal`, never an empty journal.
- The cluster's `ReplicatedJournal` ships through `ship_suffix`; its behaviour is unchanged.
- npm: `npm test` runs `packages/web/test/*.test.mjs`. Node 24 failed on the bare directory.
- CI: the browser tests start headless Chrome without its sandbox on runners, and clean up when
  Chrome fails to start. The WebAssembly job has a 45-minute limit.

### Fixed

- **Custody (found before release):**
  - A deposed writer could count its own replica toward a quorum it had lost.
  - The first rule for custodian-set changes could lose committed entries. The Lean model
    produced the counterexamples; the rule is now joint consensus, and both cases are
    regression tests.
- The WebAssembly core test compared the build's version with a literal; it now uses the
  workspace version.
- `cargo fmt --check` passes for the whole workspace again.

### Known limitations

- **Failover:** outbox progress is per node, so a host that takes an object over re-sends
  recent messages with their original message ids. Receivers deduplicate them.
- **Removed hosts:** a removed host can still read entries written before its removal.
- **Byzantine hosts:** custody detects a cheating host and keeps equivocation evidence, but
  does not prevent it.
- **`did:webvh`:** objects are resolved and verified, but Vertep does not yet create, update
  or host `did:webvh` logs. `DocumentPublisher` is the hook for that.
- **Mediator:** the WebSocket client speaks `ws://` only, and the `vt-mediator` binary runs
  one node.

## [0.1.1] - 2026-11-12

### Added

- **Packaging:** READMEs for every crate and npm package, with runnable examples (doctests
  for the crates), and crate metadata (homepage, documentation, keywords, categories).
- **Public site:** published at https://volland.github.io/vertep/ through GitHub Pages.
- **Lean:** the Shapes Equivalence Theorem. The chart shapes accept a projection exactly when
  the chart is well-formed. The verification backlog is archived.

### Changed

- **Standard errors:** `vertep-core` and `vertep-dm-cel` errors implement `Display` and
  `core::error::Error`. `Violations` wraps chart violations.
- **Traits:** `CelValue` derives `PartialEq` and `ir::Data` derives `Default`.

## [0.1.0] - 2026-11-12

The first release: milestones M0 to M6.

### Added

- **M0, SCXML core:**
  - The chart IR, the W3C SCXML interpretation algorithm as a pure `step`, the SCXML format,
    and the null and ECMAScript datamodels.
  - The RDF 1.2 projection with SHACL shapes and content-addressed chart versions.
  - The Lean 4 model with the differential oracle and `vt certify`.
  - The W3C IRP conformance suite.
- **M1, typed authoring:** one type language with JSON Schema, SHACL and TypeScript
  projections, the YAML format, the CEL datamodel, and the `statechart!` and `include_chart!`
  macros with the Rust datamodel.
- **M2, actor runtime:** virtual, invoked and stateless-worker actors, mailboxes and
  transports, and the event journal with compare-and-set appends and a transactional outbox.
  Also durable timers, chart versioning, the tiramemsu and Oxigraph stores, and the runtime
  ontology.
- **M3, agents:** deciders at `vt:decide` states, LLM and tool actors with journaled results,
  capability grants, and LLM-written plan charts run as sandboxed children.
- **M4, cluster:** pluggable membership (static, gossip, etcd, Kubernetes), shard leases with
  fencing epochs, the QUIC transport, standby replication and worker pools.
- **M5, federation:** DID identities (`did:key`, `did:peer`, `did:web`, `did:webvh`),
  authenticated origins, DID-scoped grants, and DIDComm v2 and TSP relay mailboxes and
  transports.
- **M6, WebAssembly and JavaScript:** the core and a browser shell in WebAssembly, the
  `@vertep/web` and `@vertep/dm-ecma` npm packages, and a tiramemsu journal in OPFS.

[0.1.4]: https://github.com/Volland/vertep/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/Volland/vertep/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Volland/vertep/compare/9904604...v0.1.2
[0.1.1]: https://github.com/Volland/vertep/commit/9904604
[0.1.0]: https://github.com/Volland/vertep/commit/85d5d31
