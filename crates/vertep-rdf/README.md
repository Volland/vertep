<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-rdf</h1>

<p align="center">Charts as RDF 1.2: the vt: ontology, SHACL shapes, a lossless projection and content-addressed chart versions</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-rdf"><img src="https://img.shields.io/crates/v/vertep-rdf.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-rdf"><img src="https://img.shields.io/docsrs/vertep-rdf" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-rdf.svg" alt="license"></a>
</p>

`vertep-rdf` writes a Vertep chart as an RDF 1.2 graph and rebuilds exactly the same chart from
that graph. The chart IR stays the hub and the runtime never executes RDF; the graph is a
lossless projection of it, down to the order of transitions. Because RDF is open-world, SHACL
shapes, not the ontology, define a valid chart graph, and reconstruction runs them first. The
crate ships the modular `vt:` ontology (chart, types, runtime, agent and verification modules)
and its shapes as Turtle, and a SHACL Core validator.

A chart is named by its content. Its version IRI is `urn:vertep:chart:sha256:` followed by the
SHA-256 of the RDFC-1.0 canonical N-Quads of its projection, and every node IRI is minted under
it. Formatting, comments, the surface a chart was written in (SCXML, YAML, the Rust macro) and
extra triples about the chart do not change the version. The runtime records this IRI in every
journal entry, so a replay always uses the chart that wrote the entries. Typed charts also
project their named types, under content-addressed type IRIs, with SHACL shapes and a JSON
Schema projection.

This crate builds on `vertep-core` and `oxrdf`/`oxttl`. `vertep-runtime`, `vertep-dm-rust`
(the macro's `VERSION`) and the `vt` command use it. The [`vertep`](https://crates.io/crates/vertep)
facade always includes it as `vertep::rdf`. Depend on it directly to hash, project, validate or
convert charts without the runtime.

## Install

```toml
[dependencies]
vertep-rdf = "0.1"
# To read SCXML charts, as in the example below:
vertep-scxml = "0.1"
```

The crate has no cargo features.

## Example

```rust
use vertep_rdf::{RdfFormat, chart_version, project, read_chart, validate, write_chart};

const DOOR: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" name="door" initial="closed">
  <state id="closed">
    <transition event="open" target="opened"/>
    <transition event="lock" target="locked"/>
  </state>
  <state id="opened">
    <transition event="close" target="closed"/>
  </state>
  <state id="locked">
    <transition event="unlock" target="closed"/>
  </state>
</scxml>"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chart = vertep_scxml::read_str(DOOR)?.chart;

    // The version IRI: the same one `vt hash door.scxml` prints.
    let version = chart_version(&chart);
    assert_eq!(
        version.as_str(),
        "urn:vertep:chart:sha256:1dc511b5d015deca8f320c136c262267c95b7e2cba6f76173318ded0951f63cd"
    );

    // Whitespace and attribute order are not part of the chart.
    let reformatted = DOOR.replace("  ", "    ").replace(
        r#"name="door" initial="closed""#,
        r#"initial="closed" name="door""#,
    );
    assert_eq!(chart_version(&vertep_scxml::read_str(&reformatted)?.chart), version);

    // Project to RDF, check the chart shapes, and write Turtle.
    let projected = project(&chart);
    assert_eq!(projected.version, version);
    assert!(validate(&projected.graph).conforms());
    let turtle = write_chart(&projected, RdfFormat::Turtle);

    // A comment about a state is not part of the chart: same chart, same version.
    let annotated = format!(
        "{turtle}\nc:s.opened <http://www.w3.org/2000/01/rdf-schema#comment> \"the door is open\" .\n"
    );
    let back = read_chart(annotated.as_bytes(), RdfFormat::Turtle)?;
    assert_eq!(back.chart, chart);
    assert_eq!(back.base, version.as_str());
    assert_eq!(chart_version(&back.chart), version);
    Ok(())
}
```

`chart_version` hashes the projection, so reformatting the SCXML gives the same IRI. `project`
mints every node under that IRI (`<version>#s.closed`, `<version>#s.closed:t0`, ...), with no
blank nodes, and the graph conforms to the chart shapes. `read_chart` validates and
reconstructs: triples outside the projection, like the `rdfs:comment`, are ignored, so the
annotated graph still names the same chart.

## Key types

| Item | What it is |
|---|---|
| `chart_version`, `chart_hash`, `canonical_nquads` | The content-addressed version IRI, its digest and the canonical N-Quads it hashes |
| `ChartVersion` | A `urn:vertep:chart:sha256:...` IRI: `as_str()`, `hex()`, `parse()` |
| `project`, `ProjectedChart` | The chart as an RDF graph, minted under its version |
| `reconstruct`, `Reconstructed`, `graph_version` | A graph back to the chart, after SHACL validation |
| `read_chart`, `write_chart`, `RdfFormat` | Turtle, N-Triples, N-Quads and TriG (RDF 1.2), sorted for stable output |
| `validate`, `chart_shapes`, `chart_ontology`, `ValidationReport` | The chart shapes, the chart ontology module, a SHACL Core report |
| `types` (`project_chart_types`, `TypeIri`) | Named types as RDF and SHACL under `urn:vertep:type:sha256:...` IRIs, and their JSON Schema projection |
| `RUNTIME_ONTOLOGY_TTL`, `AGENT_ONTOLOGY_TTL`, `AGENT_SHAPES_TTL`, `VERIFICATION_ONTOLOGY_TTL` | The other ontology modules and shapes as Turtle text |

## Documentation

- Website: https://volland.github.io/vertep/docs/rdf.html (the projection, the shapes, versions and the journal as RDF)
- API reference: https://docs.rs/vertep-rdf
- Examples: https://volland.github.io/vertep/examples/traffic-light.html (`vt hash` and `vt convert --to turtle`)
- Types: https://volland.github.io/vertep/docs/types.html

## License

MIT OR Apache-2.0, at your option.
