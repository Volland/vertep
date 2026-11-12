<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-dm-cel</h1>

<p align="center">The CEL datamodel for Vertep: deterministic, side-effect free, always terminating and statically type-checked</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-dm-cel"><img src="https://img.shields.io/crates/v/vertep-dm-cel.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-dm-cel"><img src="https://img.shields.io/docsrs/vertep-dm-cel" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-dm-cel.svg" alt="license"></a>
</p>

`vertep-dm-cel` implements the `cel` datamodel (`datamodel="cel"`) for Vertep charts: guards
and assignments are [CEL](https://cel.dev) expressions over `ctx` (the chart's context), `ev`
(the current event, with its payload in `ev.data`), `session` and `In("state")`. CEL has no
side effects, no clock and no randomness, every evaluation is bounded by step, size and depth
limits, maps keep a defined order, and `decimal` is exact. That makes it the datamodel for YAML
charts (it is their default) and for charts written by authors you do not fully trust,
including language models.

When a chart declares types, `instantiate` type-checks every expression against them before
the chart runs: a guard must be `bool`, a field must exist on the record it is read from, an
assignment must match the field's type. A chart with type errors is refused with all of them.
At run time, an event whose payload does not match its declared type becomes `error.execution`
instead of being delivered. `Policy::CelOnly` additionally rejects `<script>`, other
datamodels and references to registered Rust functions.

The interpreter is Vertep's own: no existing Rust CEL implementation had exact decimals,
evaluation limits and a defined map order together with `no_std`. The crate is `no_std` with
`alloc` and depends only on `vertep-core` and `regex-automata` (a linear-time `matches`). Hosts
register it through `CelRegistry`; `vertep-runtime`, `vertep-agent` (plan charts) and the `vt`
command do. The [`vertep`](https://crates.io/crates/vertep) facade does not re-export it, so
add it directly when your charts use CEL.

## Install

```toml
[dependencies]
vertep-dm-cel = "0.1"
# To read YAML charts, as in the example below:
vertep-yaml = "0.1"
vertep-core = "0.1"
```

| Feature | Default | What it turns on |
|---|---|---|
| `std` | yes | Forwards `std` to `vertep-core`; the CEL datamodel itself is always `no_std` with `alloc` |

## Example

A typed tally on CEL in YAML, run through the pure core:

```rust
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Event, EventType, StateQuery, Value, admit};
use vertep_core::null::{WithNull, create_datamodel_for};
use vertep_core::session::{Input, StartError, StartInput, start, step};
use vertep_core::validate::Violations;
use vertep_dm_cel::{CelRegistry, CelValue, Limits, instantiate};

const TALLY: &str = r#"
vertep: 1
name: tally
datamodel: cel
types:
  Add: { k: decimal }
events:
  add: Add
context:
  total: { type: decimal, init: "0" }
initial: s
states:
  s:
    on:
      add:
        - actions:
            - assign: { location: ctx.total, expr: "ctx.total + ev.data.k" }
"#;

/// `In()` is not used here, so no state is ever active for it.
struct NoStates;
impl StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

fn add(k: &str) -> Event {
    Event {
        name: "add".into(),
        type_: EventType::External,
        data: Value::Object(vec![("k".into(), Value::String(k.into()))]),
        ..Event::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chart = vertep_yaml::read_str(TALLY)?;

    // Typed load: every expression is checked against the declared types.
    let mut dm = instantiate(&chart, Limits::default())?;
    let chart = CompiledChart::new(chart).map_err(Violations)?;

    let mut session = start(&chart, StartInput::default(), &mut dm)?.session;
    for k in ["0.10", "0.20"] {
        // `admit` checks the payload against the event's declared type.
        let event = admit(&dm, add(k));
        let input = Input::event(event, Default::default());
        session = step(&chart, session, input, &mut dm)?.session;
    }
    // Decimals are exact: 0.10 + 0.20 is 0.30.
    let total = dm.evaluate("ctx.total == decimal('0.30')", &NoStates)?;
    assert_eq!(total, CelValue::Bool(true));

    // A payload that fails its declared type is never delivered.
    let wrong = Event { data: Value::Object(vec![]), ..add("1") };
    assert_eq!(admit(&dm, wrong).name, "error.execution");

    // A chart with a CEL type error is refused before it runs.
    let bad = TALLY.replace("- actions:", "- cond: \"ctx.total + 1\"\n          actions:");
    let bad = vertep_yaml::read_str(&bad)?;
    match create_datamodel_for(&WithNull(CelRegistry), &bad) {
        // "state `s` › transition 1 › cond: a guard must have type bool,
        //  but `ctx.total + 1` has type decimal"
        Err(StartError::ChartRefused { problems, .. }) => {
            assert!(problems[0].contains("a guard must have type bool"));
        }
        _ => panic!("the chart should be refused"),
    }
    Ok(())
}
```

`instantiate` gives a typed `CelDatamodel`; hosts usually go through
`create_datamodel_for(&registry, &chart)` with `CelRegistry`, which does the same and turns type
errors into `StartError::ChartRefused`. The datamodel is driven by `vertep_core::session` like any
other. `CelDatamodel::evaluate` evaluates an expression against the current context, which is
handy in tests.

## Key types

| Item | What it is |
|---|---|
| `CelDatamodel`, `CEL` | The datamodel (`vertep_core::datamodel::Datamodel`) and its name, `"cel"` |
| `CelRegistry`, `create` | Register CEL with a host; `create_for` instantiates typed charts typed |
| `instantiate`, `type_env`, `TypeEnv` | The typed load, and the CEL type environment projected from a chart's types |
| `load`, `LoadOptions`, `LoadErrors` | Parse and check every expression of a chart with explicit options |
| `Policy` (`Standard`, `CelOnly`), `check_policy` | The loading policy for untrusted charts |
| `Limits` | Evaluation bounds: steps (100,000), value size (1 MiB), nesting depth (64), expression length (64 KiB), value depth (128) |
| `CelValue`, `CelType`, `Decimal`, `Timestamp`, `Duration` | CEL values and types; exact decimals; integer timestamps and durations |
| `YAML_DEFAULT_DATAMODEL` | The datamodel of a YAML chart that names none |

## Documentation

- Website: https://volland.github.io/vertep/docs/types.html#cel (CEL, its limits and the type checker)
- API reference: https://docs.rs/vertep-dm-cel
- Examples: https://volland.github.io/vertep/examples/typed-order.html (a typed order on CEL)
- Articles: https://volland.github.io/vertep/articles/deterministic-agents.html (why charts written by models run on CEL)

## License

MIT OR Apache-2.0, at your option.
