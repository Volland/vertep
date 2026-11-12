<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-core</h1>

<p align="center">The chart IR, validation, type checker and pure W3C SCXML step function of Vertep (no_std)</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-core"><img src="https://img.shields.io/crates/v/vertep-core.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-core"><img src="https://img.shields.io/docsrs/vertep-core" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-core.svg" alt="license"></a>
</p>

`vertep-core` is the center of Vertep. It holds the chart IR (a tree of states, transitions,
executable content, invokes and data, kept in document order), the well-formedness validator,
the type checker, and the W3C SCXML interpretation algorithm as two pure functions:
`session::start` and `session::step`. They take a session as a value plus one input and return
the next session and a list of effects ("deliver `tick` in 30 s", "cancel `light-timer`").
The core has no I/O, no async and no clock. It is `no_std` with `alloc` and builds for
embedded targets.

Every other crate builds on it. `vertep-scxml`, `vertep-yaml` and the `statechart!` macro read
charts into this IR; `vertep-rdf` projects it to RDF and hashes it; the datamodel crates
(`vertep-dm-cel`, `vertep-dm-ecma`, `vertep-dm-rust`) implement its `Datamodel` trait; and
`vertep-runtime` wraps `step` in a durable actor shell. The `null` datamodel lives here.

Depend on `vertep-core` directly when you write your own host (a game loop, an embedded
controller, a test harness), a datamodel, or a chart surface. For actors, journals and storage,
use the [`vertep`](https://crates.io/crates/vertep) facade, which re-exports this crate as
`vertep::core`.

## Install

```toml
[dependencies]
vertep-core = "0.1"

# Without the standard library (alloc only):
# vertep-core = { version = "0.1", default-features = false }
```

| Feature | Default | What it turns on |
|---|---|---|
| `std` | yes | `std` support; without it the crate is `no_std` with `alloc` |
| `serde` | no | `Serialize` and `Deserialize` for the IR, sessions, inputs and effects |

## Example

A door with two states, built directly in the IR, run on the `null` datamodel:

```rust
use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Environment, Event, EventType};
use vertep_core::ir::{Chart, State, StateElement, Transition};
use vertep_core::null::NullDatamodel;
use vertep_core::session::{self, Input, StartInput};
use vertep_core::validate::{ViolationKind, Violations};

fn on(event: &str, target: &str) -> Transition {
    Transition {
        events: vec![event.into()],
        targets: vec![target.into()],
        ..Transition::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut closed = State::new("closed", StateElement::State);
    closed.transitions.push(on("open", "opened"));
    let mut opened = State::new("opened", StateElement::State);
    opened.transitions.push(on("close", "closed"));
    let door = Chart {
        name: Some("door".into()),
        states: vec![closed, opened],
        ..Chart::default()
    };

    // Validate and compile once. The compiled chart is immutable.
    let chart = CompiledChart::new(door).map_err(Violations)?;

    // start runs the first macrostep; step runs one macrostep per input.
    let mut dm = NullDatamodel::new();
    let started = session::start(&chart, StartInput::default(), &mut dm)?;
    assert_eq!(started.session.active(&chart), ["closed"]);

    let open = Event::named("open", EventType::External);
    let input = Input::event(open, Environment { now_ms: 1_000, seed: 0 });
    let outcome = session::step(&chart, started.session, input, &mut dm)?;
    assert_eq!(outcome.session.active(&chart), ["opened"]);
    assert_eq!(outcome.trace.configuration, ["opened"]);
    assert!(outcome.effects.is_empty());

    // An invalid chart is refused with every violation found, not only the first.
    let mut lost = State::new("a", StateElement::State);
    lost.transitions.push(on("go", "nowhere"));
    let violations = CompiledChart::new(Chart {
        states: vec![lost],
        ..Chart::default()
    })
    .unwrap_err();
    assert!(matches!(
        &violations[0].kind,
        ViolationKind::UnknownTarget { target } if target == "nowhere"
    ));
    Ok(())
}
```

`CompiledChart::new` runs the validator and builds the tables `step` needs. The session is a
plain value: `step` consumes it and returns the next one, with the effects the host must carry
out (`Effect::Send`, `Effect::Cancel`, `Effect::Invoke`, `Effect::Log`, ...). Time and the random
seed travel inside each `Input` through `Environment`, so the same inputs always give the same
outcome. Charts usually come from a reader instead of being built by hand; see
[`vertep-scxml`](https://crates.io/crates/vertep-scxml) and
[`vertep-yaml`](https://crates.io/crates/vertep-yaml).

## Key types

| Item | What it is |
|---|---|
| `ir::Chart`, `ir::State`, `ir::Transition`, `ir::Exec` | The chart IR, the hub every surface reads into |
| `compile::CompiledChart` | A validated chart with the indexes `step` uses |
| `validate::validate`, `validate::Violation` | The W3C SCXML structural rules, reported all at once |
| `session::start`, `session::step` | The pure interpreter: session + input -> session + effects + trace |
| `session::Session`, `session::Effect`, `session::Outcome` | Session state, effects for the host, the result of a macrostep |
| `datamodel::Datamodel`, `datamodel::DatamodelRegistry` | The contract every datamodel implements, and how hosts pick one by name |
| `null::NullDatamodel`, `null::create_datamodel_for` | The SCXML `null` datamodel; create a chart's datamodel from a registry |
| `types::check_chart`, `types::Type` | The one type language and checker shared by the loaders, the macro and the runtime |
| `tape::TapeDatamodel` | Records every datamodel request and answer, for the Lean differential oracle |
| `identity::assign_generated_ids` | Gives anonymous states stable identifiers |

## Documentation

- Website: https://volland.github.io/vertep/docs/concepts.html (configuration, macrostep, effects and the determinism rule)
- API reference: https://docs.rs/vertep-core
- Examples: https://volland.github.io/vertep/examples/traffic-light.html (an SCXML traffic light driven through `start` and `step`)
- Articles: https://volland.github.io/vertep/articles/statecharts.html

## License

MIT OR Apache-2.0, at your option.
