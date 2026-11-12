<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-yaml</h1>

<p align="center">The YAML surface for Vertep charts: a lossless mirror of the chart IR with located errors</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-yaml"><img src="https://img.shields.io/crates/v/vertep-yaml.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-yaml"><img src="https://img.shields.io/docsrs/vertep-yaml" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-yaml.svg" alt="license"></a>
</p>

`vertep-yaml` reads and writes Vertep charts as YAML 1.2. States are keyed by id under
`states`, and key order is document order, because document order is chart semantics in SCXML
(it orders states and transition priority). Every problem in a document comes back at once,
each with the file, line, column and a dotted path such as `states.checkout.on.pay[1].target`.
Reading also runs the core validator and, for typed charts, the type checker.

The writer is canonical and byte-stable, and `read(write(c)) == c` for every chart with unique
state ids. Every IR distinction has a spelling (`implicit: true`, `generated: true`, `extensions`,
a `transitions` list), so a chart converted from SCXML to YAML and back is the same chart with
the same version IRI. The reader is built on `saphyr-parser`, a YAML 1.2 parser that keeps
mapping order and source spans; YAML 1.1 parsers were rejected because they read the key `on`
as the boolean `true`.

The default datamodel of a YAML chart is `cel` (see
[`vertep-dm-cel`](https://crates.io/crates/vertep-dm-cel)). This crate builds on `vertep-core`
only. `vertep-macros` (`include_chart!`), `vertep-runtime` and the `vt` command use it. The
[`vertep`](https://crates.io/crates/vertep) facade does not re-export it, so add it directly when
you load YAML charts.

## Install

```toml
[dependencies]
vertep-yaml = "0.1"
```

The crate has no cargo features.

## Example

```rust
use vertep_yaml::{ErrorKind, ReadOptions, read, read_str, write};

const COUNTER: &str = r#"
vertep: 1
name: counter
datamodel: cel
types:
  Add: { by: int }
events: { add: Add, reset: null }
context:
  count: { type: int, init: "0" }
initial: counting
states:
  counting:
    on:
      add:
        - cond: "ev.data.by > 0"
          actions: [ { assign: { location: ctx.count, expr: "ctx.count + ev.data.by" } } ]
      reset:
        - actions: [ { assign: { location: ctx.count, expr: "0" } } ]
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read, validate and type-check.
    let chart = read_str(COUNTER)?;
    assert_eq!(chart.datamodel, "cel");
    assert_eq!(chart.states[0].id, "counting");

    // The canonical form reads back to the same chart, and is stable.
    let canonical = write(&chart)?;
    assert_eq!(read_str(&canonical)?, chart);
    assert_eq!(write(&read_str(&canonical)?)?, canonical);

    // Errors carry the file, line, column, path and a suggestion.
    let typo = "vertep: 1\nname: door\ndatamodel: \"null\"\ninitial: closed\nstates:\n  closed:\n    on: { open: opend }\n  opened: {}\n";
    let err = read(typo, &ReadOptions::default().with_file("door.yaml")).unwrap_err();
    let d = &err.diagnostics[0];
    assert_eq!(d.kind, ErrorKind::Reference);
    assert!(d.message.contains("did you mean `opened`"));
    // door.yaml:7:17: states.closed.on.open.target: unknown target `opend`; did you mean `opened`?
    assert_eq!((d.line, d.column), (7, 17));
    println!("{err}");

    // A type error: `true` assigned to an int field.
    let ill_typed = COUNTER.replace(r#"expr: "0" } }"#, r#"expr: "true" } }"#);
    let err = read_str(&ill_typed).unwrap_err();
    assert_eq!(err.diagnostics[0].kind, ErrorKind::Type);
    Ok(())
}
```

The counter is a typed chart on CEL: `types` names a record, `events` gives each event its
payload type, and `context` gives each data item a type and an initial value. `write` gives the
canonical form, which reads back to an equal chart. A misspelled target is a `Reference` error
with a suggestion, and assigning a `bool` to an `int` field is a `Type` error, both found before
the chart runs.

## Key types

| Item | What it is |
|---|---|
| `read`, `read_str` | YAML text to a `vertep_core::ir::Chart`, or a `LoadError` with every `Diagnostic` |
| `write` | A chart to canonical YAML (block style, fixed key order, byte-stable) |
| `ReadOptions` | File name for diagnostics, an `ExpressionChecker`, validation and type checking on or off (`lower_only()`) |
| `Diagnostic`, `ErrorKind` | One located problem: `Syntax`, `Schema`, `Version`, `Reference`, `Expression`, `Validation`, `Type` |
| `ExpressionChecker` | Plug a datamodel's expression parser in, so syntax errors point inside the YAML string |
| `DEFAULT_DATAMODEL` | `"cel"`, the datamodel of a YAML chart that names none |
| `SUPPORTED_VERSIONS` | The `vertep:` format versions this crate reads and writes |

Quote `"null"` when you mean the null datamodel: unquoted, YAML reads it as a null value.

## Documentation

- Website: https://volland.github.io/vertep/docs/charts.html#yaml (the YAML format, key by key)
- API reference: https://docs.rs/vertep-yaml
- Examples: https://volland.github.io/vertep/examples/typed-order.html (a typed YAML chart on CEL)
- Types: https://volland.github.io/vertep/docs/types.html

## License

MIT OR Apache-2.0, at your option.
