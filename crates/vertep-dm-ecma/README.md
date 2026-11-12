<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-dm-ecma</h1>

<p align="center">The W3C SCXML ecmascript datamodel for Vertep, on the boa JavaScript engine</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-dm-ecma"><img src="https://img.shields.io/crates/v/vertep-dm-ecma.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-dm-ecma"><img src="https://img.shields.io/docsrs/vertep-dm-ecma" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-dm-ecma.svg" alt="license"></a>
</p>

`vertep-dm-ecma` implements the `ecmascript` datamodel of W3C SCXML (Appendix B.2) on the
[boa](https://boajs.dev) engine, in pure Rust. Data items are variables of one global scope per
session; conditions, `<assign>`, `<script>` and value expressions are JavaScript. It is the
datamodel to use for existing SCXML charts, for charts that must also run in another SCXML
engine, and for the W3C conformance suite, which Vertep runs on it.

Execution stays deterministic, so actors on this datamodel replay exactly: `Math.random()` is
seeded from the input, and the current date is the timestamp carried by the input (and
journaled by the runtime), never the wall clock. Conditions run against a read-only view of the
session scope, so a condition that assigns fails. System variables are frozen. A failed
expression becomes an `error.execution` event, never a panic.

This crate implements the `Datamodel` trait of `vertep-core`. Hosts register it through
`EcmaRegistry`. The [`vertep`](https://crates.io/crates/vertep) facade re-exports it as
`vertep::ecmascript` behind its `ecmascript` feature (off by default). In the browser the same
datamodel ships as the npm package `@vertep/dm-ecma`.

## Install

```toml
[dependencies]
vertep-dm-ecma = "0.1"
vertep-core = "0.1"
```

| Feature | Default | What it turns on |
|---|---|---|
| `ecmascript` | yes | The boa engine and `EcmaDatamodel`. Without it the crate registers no datamodel, and charts that name `ecmascript` are refused. |

## Example

A counter on the `ecmascript` datamodel, built in the IR and run through the pure core:

```rust
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_core::ir::{Chart, Data, Exec, ExecKind, State, StateElement, Transition};
use vertep_core::null::create_datamodel_for;
use vertep_core::session::{Effect, Input, StartInput, start, step};
use vertep_dm_ecma::{ECMASCRIPT, EcmaRegistry};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // <data id="count" expr="0"/>
    // <transition event="add" cond="_event.data.by > 0">
    //   <assign location="count" expr="count + _event.data.by"/>
    //   <log label="count" expr="count"/>
    // </transition>
    let mut counting = State::new("counting", StateElement::State);
    counting.transitions.push(Transition {
        events: vec!["add".into()],
        cond: Some("_event.data.by > 0".into()),
        body: vec![
            Exec::new(ExecKind::Assign {
                location: "count".into(),
                expr: Some("count + _event.data.by".into()),
                content: None,
            }),
            Exec::new(ExecKind::Log {
                label: Some("count".into()),
                expr: Some("count".into()),
            }),
        ],
        ..Transition::default()
    });
    let chart = Chart {
        datamodel: ECMASCRIPT.into(),
        datamodel_explicit: true,
        data: vec![Data {
            id: "count".into(),
            src: None,
            expr: Some("0".into()),
            content: None,
            extensions: vec![],
            span: Default::default(),
        }],
        states: vec![counting],
        ..Chart::default()
    };
    let chart = CompiledChart::new(chart).map_err(|v| format!("{v:?}"))?;

    // The host picks the datamodel the chart names from its registry.
    let mut dm = create_datamodel_for(&EcmaRegistry, chart.chart()).map_err(|e| format!("{e:?}"))?;
    let mut session = start(&chart, StartInput::default(), dm.as_mut())
        .map_err(|e| format!("{e:?}"))?
        .session;

    let mut logged = Vec::new();
    for by in [2.0, -5.0, 3.0] {
        let event = Event {
            name: "add".into(),
            type_: EventType::External,
            data: Value::Object(vec![("by".into(), Value::Number(by))]),
            ..Event::default()
        };
        let outcome = step(&chart, session, Input::event(event, Default::default()), dm.as_mut())
            .map_err(|e| format!("{e:?}"))?;
        for effect in outcome.effects {
            if let Effect::Log { value: Some(v), .. } = effect {
                logged.push(v);
            }
        }
        session = outcome.session;
    }
    // The guard rejected -5, so only two transitions ran.
    assert_eq!(logged, [Value::Number(2.0), Value::Number(5.0)]);
    Ok(())
}
```

The IR above is what the SCXML in the comment reads into. With
[`vertep-scxml`](https://crates.io/crates/vertep-scxml) you would read that document instead,
passing `ReadOptions::default().with_datamodels(&vertep_scxml::Registered(&EcmaRegistry))` so
the reader accepts `datamodel="ecmascript"`. Each `<log>` comes back as an `Effect::Log` for the
host to print or drop.

## Key types

| Item | What it is |
|---|---|
| `EcmaDatamodel` | The datamodel: `new()`, or `try_new()` when engine start-up errors should be handled |
| `EcmaRegistry`, `create` | Registers `ecmascript` with a host; `None` when the feature is off |
| `ECMASCRIPT` | The datamodel name, `"ecmascript"` |
| `DEFAULT_LOOP_ITERATION_LIMIT` | How many iterations one loop may run (10,000,000) before evaluation fails, so a runaway script ends in an error |
| `SNAPSHOT_FORMAT` | The version tag of the datamodel's snapshots |

Where W3C SCXML B.2 leaves room: inline content that `JSON.parse` accepts becomes that value and
anything else a whitespace-collapsed string; values leave the engine shaped as `JSON.stringify`
would shape them, except that `undefined` is kept.

## Documentation

- Website: https://volland.github.io/vertep/docs/types.html#ecmascript (the four datamodels and when to use each)
- API reference: https://docs.rs/vertep-dm-ecma
- Browser: https://volland.github.io/vertep/docs/browser.html (`@vertep/dm-ecma` for `@vertep/web`)
- Verification: https://volland.github.io/vertep/docs/verification.html (the W3C IRP conformance suite)

## License

MIT OR Apache-2.0, at your option.
