## Why

Vertep is pitched as "like XState, but more powerful", yet through M5 it runs only as a native Rust runtime. Browsers and Node.js are where most statechart users work. The no_std core and the tokio shell behind a small runtime trait were designed so that a second shell is cheap. Tiramemsu already runs in a Web Worker with OPFS storage, so a browser actor system with a persistent, time-travelling journal is now within reach.

## What Changes

- Compile `vertep-core` to `wasm32-unknown-unknown` unchanged, with no browser-specific code paths in the core.
- Add a browser shell that implements the runtime's spawn, clock and sleep trait on the browser event loop and runs the whole actor system inside a dedicated Web Worker.
- Store actor journals with tiramemsu compiled to WebAssembly: in OPFS for durable storage, or in memory, chosen explicitly. Journal files keep the native format.
- Add a TypeScript API to:
  - create actors from SCXML, YAML or RDF chart sources
  - send events
  - subscribe to snapshots and transitions
  - make promise-based requests from outside the actor system
  - read the journal and time-travel with `asOf`
- Generate TypeScript types for events, context and state values from the IR type system.
- Offer ergonomics inspired by XState (snapshots, `matches`, `can`, subscriptions). SCXML semantics stay normative: there is no XState compatibility mode.
- Support Node.js with the same API, using worker threads and an in-memory journal with file import and export.
- Publish the packages to npm, with a bundle size budget enforced in CI.
- Run the W3C SCXML conformance suite and the Lean differential traces in headless Chrome and in Node.js.
- Add a WebSocket transport, registered as an SCXML Event I/O Processor, so browser actors and server actors exchange messages with at-least-once delivery.

## Capabilities

### New Capabilities

- `wasm-bindings`: the Vertep core and a browser shell compiled to WebAssembly. It covers worker hosting, the tiramemsu journal in OPFS or memory, the TypeScript API and generated typings, Node.js support, npm distribution, the size budget, conformance in WebAssembly, and WebSocket interop with server actors.

### Modified Capabilities

None. The capabilities this change builds on (`scxml-interpretation`, `type-system`, `actor-model`, `messaging`, `event-journal`, `storage`, `durable-timers`, `scxml-conformance`) are reused as specified. No requirement of theirs changes.

## Impact

- **Sections in tg.md:**
  - [[capabilities#WASM bindings]]
  - [[components#vertep-wasm]]
  - [[roadmap#M6 WASM and JS]]
  - [[components#vertep-runtime]] (the shell trait it implements)
  - [[components#vertep-tiramemsu]] (the journal backend it compiles)
- **Binding decisions and constraints:**
  - [[decisions#No-std core and tokio shell]]: the core must stay `no_std`, and the browser shell is a second implementation of the runtime trait.
  - [[decisions#Two storage traits with tiramemsu as default]]: the browser journal is tiramemsu on its WASM host.
  - [[decisions#Event-sourced actors]]: journals written in the browser replay natively.
  - [[decisions#SCXML semantics are normative]]: JavaScript ergonomics must not change semantics.
  - [[decisions#One IR type language]]: TypeScript becomes one more projection of the type language.
  - [[decisions#Identity, mailbox and transport layers]]: WebSocket is one more pluggable transport.
  - [[constraints#Tiramemsu has a single writer per file]]: one OPFS journal file has one writer, so a second tab must be refused.
- **New requirements:** every requirement in this change's `wasm-bindings` delta spec, cited as `openspec:wasm-bindings#<Requirement>`.
- **Code:**
  - new crate `crates/vertep-wasm`
  - new npm workspace `bindings/js` (packages `@vertep/web` and `@vertep/dm-ecma`)
  - a `vt gen ts` subcommand in `vertep-cli`
  - a WebSocket transport added on the server side to `vertep-runtime`
- **Dependencies:**
  - `wasm-bindgen`, `wasm-bindgen-futures` and `js-sys`/`web-sys`
  - tiramemsu's `tm-wasm` host (SQLite on `sqlite-wasm-rs`, with the OPFS sync-access-handle pool VFS)
  - a WebSocket client for Node.js
  - headless Chrome in CI
