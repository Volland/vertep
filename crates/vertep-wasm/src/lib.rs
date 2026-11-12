//! Vertep in WebAssembly: the unchanged `no_std` core and the actor runtime, driven by a
//! browser shell inside a dedicated worker, with journals in tiramemsu (OPFS or memory) or in
//! the in-memory store.
//!
//! The crate is the Rust half of the npm packages in `bindings/js`:
//!
//! - [`system`]: the actor system a worker hosts and the command bridge's operations on it
//!   (register and spawn charts, send, request, snapshots, `can`, journal, `asOf`, SPARQL,
//!   subscriptions). It is target-independent, so the same command script runs natively and
//!   in WebAssembly.
//! - [`store`]: explicit storage (`memory`, `opfs`; `file` natively), never a fallback.
//! - [`functions`]: named guards and actions from the host bound into the `rust`-style
//!   named-function registry, and the datamodel registry (`null`, `cel`, `rust`).
//! - [`charts`]: SCXML, YAML and RDF chart sources, validated as natively.
//! - [`trace`]: step traces for comparing the native and WebAssembly builds of the core.
//! - `conformance` (feature `conformance`, test builds only): the W3C IRP suite run from
//!   in-memory documents, for comparing WebAssembly verdicts with the native gate.
//! - `shell` and `js` (WebAssembly only): the runtime shell on `spawn_local`, `setTimeout`
//!   and `Date.now()`, and the wasm-bindgen surface the worker calls.
//! - `ws` (WebAssembly only): the WebSocket Event I/O Processor's connection to a server on
//!   the platform `WebSocket`; its frames and delivery state are `vertep_runtime::websocket`.
//!
//! The `journal` feature adds tiramemsu on its WebAssembly host (SQLite compiled to WASM); the
//! build without it holds no SQLite, so storage-free applications never load it.
//!
//! @tg: implements:: [[decisions#No-std core and tokio shell]] {confidence: partial, note: "the browser shell"}
//! @tg: implements:: [[decisions#Two storage traits with tiramemsu as default]] {confidence: partial, note: "tiramemsu on tm-wasm"}
//! @tg: implements:: [[decisions#SCXML semantics are normative]]
//! @tg: implements:: [[decisions#Event-sourced actors]] {confidence: partial, note: "journals written in WebAssembly replay natively"}

pub mod charts;
#[cfg(feature = "conformance")]
pub mod conformance;
pub mod error;
pub mod functions;
pub mod store;
pub mod system;
pub mod trace;

#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod js;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod shell;
#[cfg(all(target_family = "wasm", target_os = "unknown"))]
pub mod ws;

pub use charts::{ChartFormat, read_chart};
pub use error::{ApiError, Code, Res, Violation};
pub use functions::{ActionOutcome, Datamodels, Emitted, Implementations};
pub use store::{StorageKind, Store, StoreOptions};
pub use system::{System, SystemOptions, WebSocketOptions, event_from_json, request};
pub use trace::{Trace, TraceCase, TraceInput, trace, trace_all};

/// The crate version, which every npm package built from it must carry.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
