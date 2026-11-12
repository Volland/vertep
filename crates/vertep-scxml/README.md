<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-scxml</h1>

<p align="center">W3C SCXML 1.0 reader and writer for Vertep charts, with source locations and a loss report</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-scxml"><img src="https://img.shields.io/crates/v/vertep-scxml.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-scxml"><img src="https://img.shields.io/docsrs/vertep-scxml" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-scxml.svg" alt="license"></a>
</p>

`vertep-scxml` reads SCXML 1.0 documents into the Vertep chart IR (`vertep_core::ir::Chart`)
and writes charts back as SCXML. Reading returns the chart, the source line and column of every
element, and a `LossReport` of everything the IR does not keep. When a document is wrong, you
get every error at once, sorted by position, not only the first. The writer is the inverse of the
reader: `read(write(c)) == c` for every chart it can represent.

Vertep features ride in the `vt:` namespace (`https://w3id.org/vertep/vt#`): types, deferred
events, decide states for agents, requests, reminders. The reader keeps them as extension nodes
on their owner, so a document with `vt:` extensions is still plain SCXML that another processor
can run. The reader does no I/O: `src` attributes go through a `SourceResolver` you supply.

This crate builds on `vertep-core`. `vertep-runtime`, `vertep-macros` (`include_chart!`) and the
`vt` command use it. The [`vertep`](https://crates.io/crates/vertep) facade re-exports it as
`vertep::scxml` (feature `scxml`, on by default). Depend on it directly when you only need to
read, write or convert SCXML, without the actor runtime.

## Install

```toml
[dependencies]
vertep-scxml = "0.1"
```

The crate has no cargo features.

## Example

```rust
use vertep_scxml::{AnyDatamodel, LossKind, ReadError, ReadOptions, read, read_str, write};

const DOOR: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- A door. The comment and the XML declaration are formatting. -->
<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" name="door" initial="closed">
  <state id="closed">
    <transition event="open" target="opened"/>
  </state>
  <state id="opened" color="blue">
    <transition event="close" target="closed"/>
  </state>
</scxml>"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Read with default options: only the `null` datamodel, no `src` resolver.
    let parsed = read_str(DOOR)?;
    assert_eq!(parsed.chart.name.as_deref(), Some("door"));

    // `color` is not SCXML: the IR cannot keep it, and the loss report says so.
    let lost: Vec<_> = parsed.losses.of_kind(LossKind::UnknownAttribute).collect();
    assert_eq!(lost.len(), 1);
    assert_eq!((lost[0].span.line, lost[0].span.column), (7, 3));
    // Strict mode turns a content loss into an error.
    let strict = read(DOOR, &ReadOptions::default().strict(true));
    assert!(matches!(strict, Err(ReadError::Lossy(_))));

    // Write the chart back and read it again: the same chart.
    let xml = write(&parsed.chart)?;
    assert_eq!(read_str(&xml)?.chart, parsed.chart);

    // Every error at once, each with its position.
    let broken = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
  <state id="a"><transition event="go" target="b"/></state>
  <state id="a"/>
</scxml>"#;
    let err = read_str(broken).unwrap_err();
    for e in err.errors() {
        println!("{}:{}: {}", e.span.line, e.span.column, e.message);
    }
    // 2:17: transition target names unknown state `b`
    // 3:3: state id `a` is already declared at 2:3
    assert_eq!(err.errors().len(), 2);

    // Other datamodels must be accepted explicitly (or through a host's registry).
    let ecma = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"
        datamodel="ecmascript"><state id="s"/></scxml>"#;
    assert!(read_str(ecma).is_err());
    let options = ReadOptions::default().with_datamodels(&AnyDatamodel);
    assert_eq!(read(ecma, &options)?.chart.datamodel, "ecmascript");
    Ok(())
}
```

The door has an attribute SCXML does not define, so the reader drops it and lists it in
`losses.items`. The comment and XML declaration go to `losses.formatting`, which does not count
as a loss. The broken document has an unknown target and a duplicate id, and both come back in
one `ReadError`. `read_str` accepts only `null`; pass a `DatamodelSet` such as `AnyDatamodel` or
`Registered(&registry)` to accept others.

## Key types

| Item | What it is |
|---|---|
| `read`, `read_str` | SCXML text to `Parsed { chart, losses }`, or a `ReadError` with every error |
| `write` | A chart to SCXML text, with the `vt:` extensions in place |
| `ReadOptions` | Accepted datamodels, a `src` resolver, strict mode, reference checks |
| `LossReport`, `Loss`, `LossKind` | What the IR does not keep: content losses (`items`) and formatting (`formatting`) |
| `ReadError`, `ParseError`, `ErrorKind` | Errors with `span.line` and `span.column` |
| `DatamodelSet`, `AnyDatamodel`, `Registered` | Which `datamodel="..."` names a document may use |
| `SourceResolver`, `MapResolver`, `DeferResolver` | How `src` on `<script>`, `<data>`, `<content>` and `<invoke>` is resolved |
| `check_round_trip`, `check_chart_round_trip` | Check that a document or chart survives `write` then `read` |
| `SCXML_NS`, `VT_NS` | The SCXML and Vertep namespaces |

## Documentation

- Website: https://volland.github.io/vertep/docs/charts.html (SCXML, the loss report and the `vt:` extensions)
- API reference: https://docs.rs/vertep-scxml
- Examples: https://volland.github.io/vertep/examples/traffic-light.html
- Articles: https://volland.github.io/vertep/articles/statecharts.html

## License

MIT OR Apache-2.0, at your option.
