//! Chart sources: SCXML documents, YAML charts and RDF chart graphs, read into the IR and
//! validated exactly as the native runtime does before any actor runs them.

use vertep_core::ir::Chart;
use vertep_rdf::RdfFormat;

use crate::error::{ApiError, Res, Violation};

/// The datamodels charts may name: the ones some build of the bindings provides (`ecmascript`
/// only in the build with `@vertep/dm-ecma`; a chart naming a datamodel the running build lacks
/// fails when its actor starts).
const DATAMODELS: [&str; 4] = ["null", "cel", "rust", "ecmascript"];

/// The surface a chart source is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartFormat {
    /// An SCXML 1.0 document.
    Scxml,
    /// A YAML chart.
    Yaml,
    /// An RDF chart graph in Turtle.
    Turtle,
    /// An RDF chart graph in N-Triples.
    NTriples,
    /// An RDF chart dataset in N-Quads.
    NQuads,
    /// An RDF chart dataset in TriG.
    TriG,
}

impl ChartFormat {
    /// Parses a format name: `scxml`, `yaml`, `rdf` or `turtle`, `ntriples`, `nquads`, `trig`.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for any other name.
    pub fn parse(name: &str) -> Res<Self> {
        Ok(match name {
            "scxml" | "xml" => ChartFormat::Scxml,
            "yaml" | "yml" => ChartFormat::Yaml,
            "rdf" | "turtle" | "ttl" => ChartFormat::Turtle,
            "ntriples" | "nt" => ChartFormat::NTriples,
            "nquads" | "nq" => ChartFormat::NQuads,
            "trig" => ChartFormat::TriG,
            other => {
                return Err(ApiError::arg(format!(
                    "unknown chart format {other:?} (scxml, yaml, rdf, turtle, ntriples, nquads, trig)"
                )));
            }
        })
    }

    /// Guesses the format of a source from its first characters: `<scxml` or `<?xml` is SCXML,
    /// `@prefix` or `<http` is Turtle, anything else YAML.
    pub fn sniff(source: &str) -> Self {
        let t = source.trim_start_matches('\u{feff}').trim_start();
        if t.starts_with("<?xml") || t.starts_with("<scxml") || t.starts_with("<!--") {
            ChartFormat::Scxml
        } else if t.starts_with("@prefix") || t.starts_with("PREFIX") || t.starts_with("<http") {
            ChartFormat::Turtle
        } else {
            ChartFormat::Yaml
        }
    }
}

fn violation(message: String, line: u32, column: u32, path: Option<String>) -> Violation {
    Violation {
        message,
        line,
        column,
        path,
    }
}

/// Reads `source` in `format` into the chart IR and validates it.
///
/// # Errors
///
/// [`crate::error::Code::ChartInvalid`] listing every violation with its location.
// @tg: implements:: [[openspec:wasm-bindings#Actor Creation From Chart Sources]]
pub fn read_chart(source: &str, format: ChartFormat) -> Res<Chart> {
    let chart = match format {
        ChartFormat::Scxml => match vertep_scxml::read(
            source,
            &vertep_scxml::ReadOptions::new().with_datamodels(&DATAMODELS),
        ) {
            Ok(p) => p.chart,
            Err(e) => {
                let mut vs: Vec<Violation> = e
                    .errors()
                    .iter()
                    .map(|p| violation(p.message.clone(), p.span.line, p.span.column, None))
                    .collect();
                if vs.is_empty() {
                    vs.push(violation(format!("{e:?}"), 0, 0, None));
                }
                return Err(ApiError::chart(vs));
            }
        },
        ChartFormat::Yaml => vertep_yaml::read_str(source).map_err(|e| {
            ApiError::chart(
                e.diagnostics
                    .iter()
                    .map(|d| {
                        violation(
                            d.message.clone(),
                            d.line,
                            d.column,
                            (!d.path.is_empty()).then(|| d.path.clone()),
                        )
                    })
                    .collect(),
            )
        })?,
        rdf => {
            let f = match rdf {
                ChartFormat::NTriples => RdfFormat::NTriples,
                ChartFormat::NQuads => RdfFormat::NQuads,
                ChartFormat::TriG => RdfFormat::TriG,
                _ => RdfFormat::Turtle,
            };
            vertep_rdf::read_chart(source.as_bytes(), f)
                .map_err(|e| ApiError::chart(vec![violation(e.to_string(), 0, 0, None)]))?
                .chart
        }
    };
    validate(&chart)?;
    Ok(chart)
}

/// Runs the core validator on `chart`.
///
/// # Errors
///
/// [`crate::error::Code::ChartInvalid`] listing every violation with its location.
pub fn validate(chart: &Chart) -> Res<()> {
    vertep_core::validate::validate(chart).map_err(|vs| {
        ApiError::chart(
            vs.iter()
                .map(|v| {
                    violation(
                        format!("{:?}", v.kind),
                        v.span.line,
                        v.span.column,
                        Some(format!("{:?}", v.node)),
                    )
                })
                .collect(),
        )
    })
}

/// The content-addressed version IRI of a chart, the same function the native runtime and
/// `vt hash` use.
pub fn version_of(chart: &Chart) -> String {
    vertep_rdf::chart_version(chart).as_str().to_owned()
}

/// The chart as RDF (Turtle), for tests that load the same chart from both surfaces.
pub fn to_turtle(chart: &Chart) -> String {
    let projected = vertep_rdf::project(chart);
    vertep_rdf::write_chart(&projected, RdfFormat::Turtle)
}
