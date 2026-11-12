<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-dm-rust</h1>

<p align="center">Statecharts in Rust: the statechart! macro, typed generated APIs, and the rust datamodel for Vertep</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-dm-rust"><img src="https://img.shields.io/crates/v/vertep-dm-rust.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-dm-rust"><img src="https://img.shields.io/docsrs/vertep-dm-rust" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-dm-rust.svg" alt="license"></a>
</p>

`vertep-dm-rust` lets you write a W3C SCXML statechart in Rust and run it with typed events,
typed context and Rust functions as guards and actions. The `statechart!` macro (and
`include_chart!` for SCXML or YAML files) checks the chart at compile time with the same
validator and type checker the runtime uses: a typo in a target, an unhandled event under
`#[deny(unhandled_events)]` or an action with the wrong payload type is a compile error at the
offending tokens. Next to the chart it generates `Ctx`, `Event`, `State`, a struct or enum per
named type, `CHART`, `VERSION`, `bind()` and `Machine`.

This crate is also the `rust` datamodel: the context is the generated `Ctx` struct, and guards
and actions are functions registered by IRI (`vt:guard/<chart>/<fn>`, `vt:action/<chart>/<fn>`).
A guard is `fn(&Ctx, &P) -> bool`, so it cannot change anything. An action is
`fn(&mut Ctx, &P, &mut Emit)`, `fn(&mut Ctx, &P)` or `fn(&mut Ctx, &P) -> Vec<E>`; its only
effects are context changes and emitted events, which become `<raise>` and `<send>` in the step.
Because binding is by IRI, the same chart read from SCXML, YAML or RDF binds to the same
compiled functions and has the same version IRI.

The macros live in `vertep-macros` and are re-exported here, so this is the only dependency a
crate needs. It builds on `vertep-core` (the step function) and `vertep-rdf` (`VERSION`, the
same IRI `vt hash` prints). The [`vertep`](https://crates.io/crates/vertep) facade does not
re-export it; add it directly.

## Install

```toml
[dependencies]
vertep-dm-rust = "0.1"
```

The crate has no cargo features.

## Example

A coin-operated turnstile, from `examples/rust-macro` in the repository:

```rust
use vertep_dm_rust::{MachineError, statechart};

statechart! {
    name: turnstile,
    types { Coin = { cents: int } }
    context { credit: int = 0, price: int = 50, passes: int = 0 }
    events { coin: Coin, push, shutdown }

    initial service;
    state service {
        initial locked;
        on shutdown => off;
        state locked {
            // Targetless: stay locked and count the coin.
            on coin / add_credit;
            // Eventless: unlock as soon as there is enough credit.
            always if paid => unlocked;
        }
        state unlocked {
            on coin / add_credit;
            on push => locked / let_through;
        }
    }
    final off;
}

/// Guard: the context and the payload, read-only.
fn paid(ctx: &Ctx, _: &()) -> bool {
    ctx.credit >= ctx.price
}

/// Action: may change the context; `Emit` would let it raise or send events.
fn add_credit(ctx: &mut Ctx, coin: &Coin, _: &mut Emit) {
    ctx.credit += coin.cents;
}

/// Action without an emitter: the shorter signature is accepted too.
fn let_through(ctx: &mut Ctx, _: &()) {
    ctx.credit -= ctx.price;
    ctx.passes += 1;
}

fn main() -> Result<(), MachineError> {
    assert_eq!(
        *VERSION,
        "urn:vertep:chart:sha256:0b1614ef361262f9f6fec5189fa5834756f37808a3a466f81a9f83d6ed20e868"
    );

    // `start` runs the first macrostep and returns its effects (none here).
    let (mut m, _effects) = Machine::start("turnstile-1")?;
    assert_eq!(m.active(), [State::Service, State::Locked]);

    m.send(Event::Coin { cents: 20 })?;
    m.send(Event::Push)?; // still locked: 20 is not enough
    assert!(m.is_in(State::Locked));

    m.send(Event::Coin { cents: 50 })?; // 70 >= 50: the eventless transition unlocks
    assert_eq!(m.active(), [State::Service, State::Unlocked]);

    m.send(Event::Push)?;
    assert_eq!((m.context().credit, m.context().passes), (20, 1));

    m.send(Event::Shutdown)?;
    assert!(m.is_terminated(), "`off` is a top-level final state");
    Ok(())
}
```

The macro checks the chart while the crate compiles and generates the typed API used in
`main`. `Event::Coin { cents: 20 }` carries the `Coin` payload; `Event::Coin { cents: "20" }`
would not compile. `paid`, `add_credit` and `let_through` are bound by name, and a function with
the wrong signature is a type error at its name in the chart. `VERSION` is the chart's
content-addressed version IRI, equal to what `vt hash` prints for the same chart in SCXML or
YAML.

## Key types

| Item | What it is |
|---|---|
| `statechart!` | Write a chart in Rust tokens on the `rust` datamodel |
| `include_chart!("path.scxml")` | Read an SCXML or YAML file (on the `rust` datamodel) at compile time, relative to `CARGO_MANIFEST_DIR`, and generate the same API |
| Generated: `Ctx`, `Event`, `State`, `CHART`, `VERSION`, `NAME`, `bind()`, `Machine` | The typed API next to each chart |
| `Machine` | A typed session: `start`, `send`, `active`, `is_in`, `context`, `is_terminated`, `step` |
| `RustDatamodel`, `Context`, `DynContext`, `RUST_DATAMODEL` | The `rust` datamodel over a generated or dynamic context |
| `Registry`, `GuardFn`, `IntoAction`, `chart_functions` | Guard and action functions by IRI; duplicates and missing IRIs are errors |
| `Emit`, `SendOptions`, `Sys` | Raise or send events from an action; typed system variables |
| `Decimal`, `Datetime`, `Iri`, `Value` | Rust types for `decimal`, `datetime`, `iri` and `value` |
| `lowering` | How guards, actions and context appear in the IR, so every surface binds the same way |

In the macro, types go in `types { ... }`, the context in `context { name: type = literal }` and
events in `events { name: Type }`. Unions become enums you can match exhaustively, records
become structs, and `optional<T>` becomes `Option<T>`.

## Documentation

- Website: https://volland.github.io/vertep/docs/charts.html#macro (the macro, its grammar and its compile errors)
- API reference: https://docs.rs/vertep-dm-rust
- Examples: https://volland.github.io/vertep/examples/rust-macro.html
- Types: https://volland.github.io/vertep/docs/types.html#rust

## License

MIT OR Apache-2.0, at your option.
