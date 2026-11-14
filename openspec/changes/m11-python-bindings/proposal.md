## Why

Vertep reaches Rust through crates and browsers and Node.js through `@vertep/web`. The people who build data pipelines, notebooks, agents and prototypes work in Python, and today they cannot use a Vertep chart without writing Rust or running a Node.js worker. The JavaScript bindings already proved the useful shape: a thin client over the actor system's command bridge, so the semantics, the journal format and the tests are shared. Python should get the same, installable with `pip install vertep`.

## What Changes

- **New crate `vertep-py`:** a PyO3 extension module, `vertep._vertep`, with one class, `Core`, over `vertep_wasm::System`, the actor system behind the JavaScript bindings. It is built as an `abi3` extension (one wheel per platform for every CPython from 3.9) and is not published to crates.io.
  - Python callables bind as named guards and actions by IRI, with the JavaScript contract.
  - Time is explicit: a system clock or a virtual clock that moves only when Python moves it.
  - The store is memory or a native tiramemsu file, the format the Rust runtime and `@vertep/web` open.
- **New Python package `vertep`** in `bindings/python`: `ActorSystem`, `Actor`, `Snapshot`, `Subscription`, typed errors with the bridge's stable codes, type stubs and a `py.typed` marker.
- **Examples** in `examples/python`, each checked against its recorded output by the test suite.
- **Docs:** `site/docs/python.html` and a README that doubles as the PyPI page.
- **Publishing:** a CI job on Python 3.9 and 3.13, a release workflow that builds wheels for five platforms and an sdist, smoke-tests each wheel and uploads with PyPI Trusted Publishing, a version gate that ties the package to the Rust workspace, and `bindings/python/RELEASING.md`.

Out of scope for this change: `asyncio`, request/reply, the WebSocket transport, agents and deciders, federation, the cluster, and the ECMAScript datamodel in the published wheels (it is a cargo feature).

## Capabilities

### New Capabilities

- `python-bindings`: Vertep charts, actors, journals and time travel from Python, and the distribution of the package.

### Modified Capabilities

None.

## Impact

- New crate `crates/vertep-py` (workspace member, `publish = false`) depending on `vertep-wasm` with its `journal` feature. `vertep-wasm` is unchanged.
- New directories `bindings/python` and `examples/python`; new workflow `.github/workflows/python-release.yml`; a `python` job in `ci.yml`.
- tg.md sections touched: `components#vertep-py` (new), `capabilities#Python bindings` (new), `roadmap#M11 Python bindings` (new), `decisions` (D1-D4), `constraints#PyPI releases are immutable`, `components#vertep-wasm` (consumed by the new crate), `capabilities#WASM bindings`.
- Decisions that bind the area: [[decisions#No-std core and tokio shell]], [[decisions#Event-sourced actors]], [[decisions#Implementations are a module imported by the worker]], [[decisions#Browser storage is explicit, exclusive and native-format]].
- Adds requirements `[[openspec:python-bindings#Native Extension Module]]`, `[[openspec:python-bindings#Actor Creation And Event Sending From Python]]`, `[[openspec:python-bindings#Snapshots And Subscriptions]]`, `[[openspec:python-bindings#Journal Inspection And Time Travel]]`, `[[openspec:python-bindings#Explicit Time]]`, `[[openspec:python-bindings#Durable File Storage]]`, `[[openspec:python-bindings#Typed Errors]]`, `[[openspec:python-bindings#Python Guards And Actions]]`, `[[openspec:python-bindings#Runnable Examples]]` and `[[openspec:python-bindings#Package Publishing]]`.
