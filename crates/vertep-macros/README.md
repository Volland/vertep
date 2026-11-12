<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-macros</h1>

<p align="center">The statechart! and include_chart! procedural macros: Vertep charts checked at compile time</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-macros"><img src="https://img.shields.io/crates/v/vertep-macros.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-macros"><img src="https://img.shields.io/docsrs/vertep-macros" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-macros.svg" alt="license"></a>
</p>

`vertep-macros` holds two procedural macros. `statechart!` takes a chart written in Rust
tokens; `include_chart!` reads an SCXML (`.scxml`, `.xml`) or YAML (`.yaml`, `.yml`) file at
compile time. Both run one pipeline inside the compiler: build the chart IR, check it with the
same well-formedness validator and type checker the runtime uses, report unreachable states and
unhandled events, and generate a typed API (`Ctx`, `State`, `Event`, `CHART`, `VERSION`,
`bind()`, `Machine`). Every finding is a compile error or warning at the offending tokens. The
macros never read a store and never go through RDF.

**Do not depend on this crate directly.** The generated code refers to `::vertep_dm_rust`, which
re-exports both macros together with the runtime side they need (the `rust` datamodel, the
function registry, `Machine`). Add [`vertep-dm-rust`](https://crates.io/crates/vertep-dm-rust)
and import the macros from there. This crate builds on `vertep-core`, `vertep-scxml` and
`vertep-yaml`, and on `syn`, `quote` and `proc-macro2`.

## Install

```toml
[dependencies]
vertep-dm-rust = "0.1"   # re-exports statechart! and include_chart! from this crate
```

The crate has no cargo features.

## Example

<!--
This block is not compiled as a doctest of vertep-macros: the generated code needs
`vertep_dm_rust`, which is not a dependency of this crate. It is the crate-level doctest of
vertep-dm-rust (crates/vertep-dm-rust/src/lib.rs), which runs with `cargo test --doc -p
vertep-dm-rust`; keep the two in step.
-->

```rust,ignore
use vertep_dm_rust::{Decimal, statechart};

statechart! {
    name: order,
    types { Pay = { amount: decimal } }
    context { total: decimal = 0 }
    events { start, pay: Pay }
    initial idle;
    state idle { on start => cart; }
    state cart { on pay if positive => paid / record; }
    final paid;
}

fn positive(_: &Ctx, ev: &Pay) -> bool { ev.amount > Decimal::ZERO }
fn record(ctx: &mut Ctx, ev: &Pay, _: &mut Emit) { ctx.total = ev.amount; }

let (mut m, _) = Machine::start("s1").unwrap();
m.send(Event::Start).unwrap();
m.send(Event::Pay { amount: "12.5".parse().unwrap() }).unwrap();
// `paid` is a top-level final state: the session has ended.
assert!(m.is_terminated());
assert_eq!(m.context().total.to_string(), "12.5");
```

The macro generates `Ctx { total: Decimal }`, a `Pay` struct, `Event::{Start, Pay { amount }}`,
`State::{Idle, Cart, Paid}` and a `Machine` over them. The guard `positive` and the action
`record` are ordinary functions bound by name; their signatures are checked against the payload
type of `pay`.

The same chart can live in a file. `include_chart!` takes a path relative to the crate's
`CARGO_MANIFEST_DIR`, rebuilds the crate when the file changes, and generates the same API. The
file must use the `rust` datamodel:

```rust,ignore
use vertep_dm_rust::{Decimal, include_chart};

include_chart!("charts/order.yaml");
```

Mistakes are compile errors at the right tokens. From the compile-fail tests in
`crates/vertep-dm-rust/tests/ui/`:

```text
error: unknown target state `paymnt`; did you mean `payment`?
 --> tests/ui/unknown_target.rs:7:28
  |
7 |     state cart { on pay => paymnt; }
  |                            ^^^^^^
```

## Grammar at a glance

| Item | Form |
|---|---|
| Chart header | `name: order,` (required), `datamodel: rust,`, `binding: early` or `late` |
| Lints | `#[deny(unreachable_states)]`, `#[allow(unhandled_events)]` (both warn by default) |
| Declarations | `types { Pay = { amount: decimal } }`, `context { total: decimal = 0 }`, `events { start, pay: Pay }` |
| States | `state s;`, `state s { ... }`, `parallel p { ... }`, `final f;`, `history h deep => t;` |
| State body | `initial a;`, `data { ... }`, `entry a, b;`, `exit a;`, `invoke "type" src "..." id "..." autoforward;` |
| Transitions | `on pay if positive => payment / record_total;`, `always if ready => running;`, `on coin / add_credit;` |
| Guards | `if name`, `if !name`, `if In(state)` |
| Actions | a function name, `raise e`, `send e to "..." after "..." type "..." id "..."`, `log "..."`, `cancel "..."` |

The full grammar is at the top of `src/parse.rs`.

## Documentation

- Website: https://volland.github.io/vertep/docs/charts.html#macro (the macro, `include_chart!` and the errors)
- API reference: https://docs.rs/vertep-macros
- Examples: https://volland.github.io/vertep/examples/rust-macro.html
- The runtime side: https://docs.rs/vertep-dm-rust

## License

MIT OR Apache-2.0, at your option.
