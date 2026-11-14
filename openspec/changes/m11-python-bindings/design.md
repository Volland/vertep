## Context

`vertep-wasm` holds `System`, the actor system the JavaScript worker hosts, with a JSON command bridge (`register`, `spawn`, `send`, `snapshot`, `can`, `journal`, `asOf`, `query`, `subscribe`, ...). It is target-independent: the native helper `vertep-wasm-native` runs the same command scripts as the WebAssembly build, and the native journal store opens the file the Rust runtime opens. `@vertep/web` is a typed client over it. The native Rust runtime (`vertep-runtime` on tokio) is richer, with agents, federation and the cluster, but is async, `Send`-bounded in places and a much larger surface to bind.

## Goals / Non-Goals

**Goals:**

- Python behaves as `@vertep/web` does: same charts, same events, same journal, same error codes.
- `pip install vertep` works without a Rust toolchain on mainstream platforms.
- Deterministic tests: a clock the test controls.
- A journal written from Python replays in Rust and JavaScript, and the reverse.

**Non-Goals:**

- Binding the whole native runtime. Agents, federation and the cluster stay Rust (and, for agents over HTTP, are a later change).
- An `asyncio` API in this change.
- A wheel per Python version.

## Decisions

### D1. Python bindings wrap the command bridge

`vertep-py` exposes `Core.command(op, json) -> json` over `vertep_wasm::System`; the ergonomic API is plain Python over it. Alternatives rejected: binding `vertep-runtime` directly (a second semantics layer to keep equal to the JavaScript one, and an async tokio runtime inside every Python process), and generating bindings from the type language (it describes charts, not the actor API). The bridge is the one place both bindings and the native tests already agree.

Reasons: [[decisions#One worker hosts the core, the shell and the journal]] made the bridge the stable seam; [[decisions#JavaScript ergonomics follow XState and semantics follow SCXML]] set the event and snapshot shapes Python mirrors.

### D2. The clock moves only inside calls

A `Core` is on the system clock or on a virtual clock. On the system clock the shell's clock is set to the wall clock before every call and `wait` sleeps, in slices, with the interpreter free and Ctrl-C able to interrupt. On the virtual clock `advance` moves time in one jump and fires due timers in order. There is no background thread, so nothing runs while Python is not inside a call, and a test never sleeps.

Reasons: [[decisions#The browser shell implements the runtime shell trait]] already separates the step from the clock; the shell is virtual-time with the clock driven by the binding.

### D3. A system belongs to one thread and the API is synchronous

Datamodels need not be `Send`, so `Core` is a PyO3 `unsendable` class: another thread gets an error instead of a data race. The API returns when the macrostep is done. An `asyncio` layer can be added over the same core later.

### D4. One abi3 wheel per platform, published by Trusted Publishing

The extension is built against the stable ABI (`abi3-py39`), so a platform has one wheel for every CPython from 3.9. An sdist covers other platforms. The release workflow builds Linux (x86_64, aarch64), macOS (x86_64, arm64) and Windows (x64), smoke-tests each wheel in a clean environment and uploads with OpenID Connect, with no stored token and a required reviewer on the `pypi` environment.

### D5. Python callables are the implementations module

A `{iri: callable}` mapping binds guards and actions like the module the JavaScript worker imports, with the same contract: `(context, event)` in, a bool or an outcome out, the event as `{type, ...fields, _meta}`. Impurity is reported by `dev=True` replay as an `ImpurityWarning`.

## Risks / Trade-offs

- **Platform coverage unproven until the first dry run.** Only the macOS arm64 build is exercised on the author's machine; the release workflow's dry run on TestPyPI is the gate for the others.
- **`vertep-wasm` pulls native-only dependencies** (`vertep-testkit`, `proptest`, `tokio`, the WebSocket server) because they are unconditional native dependencies of that crate; they are not linked into the wheel's code paths but they lengthen the build. Gating them behind a feature of `vertep-wasm` is a follow-up that changes a released crate's manifest.
- **Numbers.** The bridge's CEL context decoding yields JSON numbers, so integers arrive as floats.
- **PyPI releases are immutable.** A bad release is yanked, never replaced; the dry run exists for that reason.

## Migration Plan

Additive. Nothing existing changes. First release: reserve the name with a pending trusted publisher, dry-run to TestPyPI, tag.
