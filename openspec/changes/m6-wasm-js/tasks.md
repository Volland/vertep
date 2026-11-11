## 1. Core and build setup

- [x] 1.1 Add the `wasm32-unknown-unknown` target to CI and build `vertep-core` for it with default features; fail the build on any `cfg(target_arch)` inside `vertep-core`
- [x] 1.2 Create the crate `crates/vertep-wasm` with `wasm-bindgen`, `wasm-bindgen-futures`, `js-sys` and `web-sys`, depending on `vertep-runtime` and `vertep-tiramemsu`
- [x] 1.3 Add a native-versus-WASM step comparison test that runs the same charts and journaled inputs on both targets and compares configurations, datamodels and effects

## 2. Browser shell

- [x] 2.1 Implement the runtime spawn, clock and sleep trait on `spawn_local`, `setTimeout` and `Date.now()` for browsers
- [x] 2.2 Implement the same trait for Node.js worker threads
- [x] 2.3 Fire overdue durable timers once at actor system start, after journals are opened
- [x] 2.4 Add a test that replays a browser-recorded journal under a different wall clock and gets identical states

## 3. Worker hosting and command bridge

- [x] 3.1 Define the main-thread to worker message protocol (commands, replies, notifications, errors with codes)
- [x] 3.2 Host the actor system in a dedicated worker; make every client call return a Promise or a subscription
- [x] 3.3 Load the implementations module inside the worker and register its exports as named guards and actions
- [x] 3.4 Reject pending Promises with `WorkerTerminated` when the worker dies; fail later calls the same way
- [x] 3.5 Add a development mode that double-steps each macrostep and reports guard or action impurity (done as a replay check: after each new entry the journal is replayed and compared with the live state, which reports a guard or action whose second evaluation differs)

## 4. Journal storage

- [x] 4.1 Open `vertep-tiramemsu` on the `tm-wasm` host with `opfs` or `memory` storage chosen explicitly and the rollback journal
- [x] 4.2 Take a Web Lock per store path before opening OPFS; return `StoreLocked` to a second holder
- [x] 4.3 Return `StorageUnavailable` when OPFS sync access handles are missing, without opening a memory store
- [x] 4.4 Expose journal file export and import; add a round-trip test that replays a browser journal natively and a native journal in the browser
- [x] 4.5 Lazy-load the SQLite module on first journaled store open (the worker loads the build with SQLite only for journaled stores; plain memory stores load the core build)

## 5. TypeScript API

- [ ] 5.1 Implement `createActorSystem` with storage, path, implementations and transports options (OPEN: storage, path, implementations, dev, import and charts work; the transports option is accepted but there is no transport until section 7)
- [x] 5.2 Implement actor creation from SCXML, YAML and RDF sources with validation errors as `ChartInvalid` and chart version IRIs in snapshots
- [x] 5.3 Implement virtual actor addressing by chart type and key with activation on first message
- [x] 5.4 Implement `send` with message ids and deduplication, and `request` with timeout and `RequestTimeout`
- [x] 5.5 Implement snapshot and transition subscriptions, delivered once per macrostep in journal order, and unsubscribe
- [x] 5.6 Implement `journal`, `asOf` by position or transaction time, and SPARQL queries on the graph store as of a time
- [x] 5.7 Implement `getSnapshot`, `matches` and a side-effect-free `can` that reports enabled or deferred events

## 6. Generated TypeScript types

- [x] 6.1 Add the TypeScript projection of the IR type language (events union, context interface, state identifier union, typed `createActor`)
- [x] 6.2 Add `vt gen ts` to `vertep-cli`
- [x] 6.3 Add a property test that TypeScript and JSON Schema projections accept the same values
- [x] 6.4 Add compile-fail tests that ill-typed events are rejected by the TypeScript checker; check untyped charts fall back to a dynamic JSON value type

## 7. WebSocket interop

- [ ] 7.1 Implement the WebSocket Event I/O Processor in `vertep-wasm` with envelopes (target, origin, message id, event, payload) and acknowledgements (OPEN: not started: the WebSocket client, server endpoint, redelivery and credentials need a server-side WebSocket library in vertep-runtime and an end-to-end harness; deferred)
- [ ] 7.2 Implement the server side of the WebSocket processor in `vertep-runtime` (OPEN: not started, see 7.1)
- [ ] 7.3 Redeliver unacknowledged outbox entries after reconnect; deduplicate by message id on receipt (OPEN: not started, see 7.1)
- [ ] 7.4 Add the pluggable credential provider and `TransportRefused` errors (OPEN: not started, see 7.1; `TransportRefused` exists as an error code)
- [ ] 7.5 Add an integration test: a browser actor and a server actor exchange events across a forced disconnect, each processing every message once (OPEN: not started, see 7.1)

## 8. Conformance, packaging and budget

- [ ] 8.1 Run the W3C SCXML conformance suite in headless Chrome and Node.js with the shared allow-list; fail on unexpected passes or failures (OPEN: the IRP harness in vertep-conformance reads the suite from files and is not ported to WebAssembly; the generated differential traces do run in headless Chrome and Node.js (8.2))
- [x] 8.2 Run the Lean differential traces in WebAssembly and compare them with native traces (the Rust/WebAssembly side: generated cases run in WebAssembly (Node.js and headless Chrome) give byte-identical oracle records to native, and the native records are checked equal to the ones vertep-conformance hands the Lean oracle; the Lean replay itself stays in the native nightly job)
- [x] 8.3 Create the npm workspace `bindings/js` with `@vertep/web` and `@vertep/dm-ecma` as ES modules with declarations and the worker entry
- [x] 8.4 Fail releases whose package version differs from the workspace crate version
- [x] 8.5 Add the CI size check: 600 KB gzip without storage, 2.5 MB gzip with the tiramemsu journal (the check runs in CI and in the tests; it currently fails: the core build is 1.14 MB gzip against 600 KB, the journal build 2.18 MB within 2.5 MB)
- [ ] 8.6 Publish to npm under the `next` tag, then `latest` after the budget and round-trip tests hold (OPEN: not published, by instruction; the packages carry publishConfig tag next)

## 9. Traceability and docs

- [x] 9.1 Annotate the code that implements each requirement with `// @tg: implements:: [[openspec:wasm-bindings#<Requirement>]]` above the function, type or file, with `{confidence: partial}` where it applies
- [x] 9.2 Annotate each test that proves a scenario with `// @tg: verifies:: [[openspec:wasm-bindings#<Requirement>#<Scenario>]]` directly above the test call
- [x] 9.3 Record the design decisions D1 to D8 in `tg.md/decisions.md` as `Decision` sections with `motivated_by` edges, and add any new constraint (for example, OPFS exclusivity) to `tg.md/constraints.md` with `constrains` edges to the requirements (the OPFS constraint constrains the new decisions; doc edges to openspec requirements are not resolvable in tg 0.22, so the requirements are named in prose)
- [x] 9.4 DONE AS FRONTMATTER (tg 0.22 does not resolve doc links to openspec requirements; capabilities.md lists wasm-bindings in its openspec frontmatter and code carries @tg annotations), was: Update `tg.md/capabilities.md#WASM bindings` with `specified_by` edges to its requirements, and keep `wasm-bindings` listed in that file's `openspec:` frontmatter
- [ ] 9.5 Run `tg check`, `tg ontology check` and `tg trace wasm-bindings --gaps`, and close every gap they report (OPEN: tg check and tg ontology check pass; tg trace wasm-bindings reports 14 of 15 implemented and 13 verified: the WebSocket requirement (section 7) and the W3C suite scenario (8.1) remain)
