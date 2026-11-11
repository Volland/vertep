//! Chart surfaces: detecting, reading and writing SCXML, YAML and the RDF syntaxes.

use std::fmt;
use std::path::Path;

use vertep_core::ir::Chart;
use vertep_rdf::RdfFormat;
use vertep_scxml::{AnyDatamodel, DeferResolver, LossKind, ReadOptions};

/// A surface a chart can be written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// SCXML 1.0 with the `vt:` extensions.
    Scxml,
    /// The YAML chart format.
    Yaml,
    /// RDF 1.2 Turtle.
    Turtle,
    /// RDF 1.2 TriG.
    TriG,
    /// RDF 1.2 N-Quads.
    NQuads,
    /// RDF 1.2 N-Triples.
    NTriples,
}

impl Surface {
    /// The surface named on the command line (`--to`).
    pub fn from_name(name: &str) -> Option<Surface> {
        Some(match name {
            "scxml" => Surface::Scxml,
            "yaml" => Surface::Yaml,
            "turtle" | "ttl" => Surface::Turtle,
            "trig" => Surface::TriG,
            "nquads" | "nq" => Surface::NQuads,
            "ntriples" | "nt" => Surface::NTriples,
            _ => return None,
        })
    }

    /// The surface of a file, from its extension or else its content.
    pub fn detect(path: &Path, content: &str) -> Surface {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        match ext.as_deref() {
            Some("scxml" | "xml") => return Surface::Scxml,
            Some("yaml" | "yml") => return Surface::Yaml,
            Some("ttl") => return Surface::Turtle,
            Some("trig") => return Surface::TriG,
            Some("nq") => return Surface::NQuads,
            Some("nt") => return Surface::NTriples,
            _ => {}
        }
        let t = content.trim_start_matches('\u{feff}').trim_start();
        if t.starts_with('<') && (t.starts_with("<?xml") || t.contains("<scxml")) {
            Surface::Scxml
        } else if t.lines().any(|l| l.starts_with("vertep:")) {
            Surface::Yaml
        } else if t.starts_with("@prefix") || t.starts_with("PREFIX") || t.contains(" {") {
            Surface::TriG
        } else {
            Surface::NQuads
        }
    }

    fn rdf(self) -> Option<RdfFormat> {
        match self {
            Surface::Turtle => Some(RdfFormat::Turtle),
            Surface::TriG => Some(RdfFormat::TriG),
            Surface::NQuads => Some(RdfFormat::NQuads),
            Surface::NTriples => Some(RdfFormat::NTriples),
            Surface::Scxml | Surface::Yaml => None,
        }
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Surface::Scxml => "scxml",
            Surface::Yaml => "yaml",
            Surface::Turtle => "turtle",
            Surface::TriG => "trig",
            Surface::NQuads => "nquads",
            Surface::NTriples => "ntriples",
        })
    }
}

/// A chart read from a file.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The chart.
    pub chart: Chart,
    /// The surface it was read from.
    pub surface: Surface,
    /// What reading dropped from the source document (SCXML content losses).
    pub lost: Vec<String>,
}

/// How to read a YAML chart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    /// Lower and validate structure only.
    Structure,
    /// Also run the type checker (YAML reports type errors with their YAML location).
    Types,
}

static ANY: AnyDatamodel = AnyDatamodel;

/// `file:line:column: message`, with the position omitted when unknown.
pub fn located(file: &str, line: u32, column: u32, message: &str) -> String {
    if line == 0 {
        format!("{file}: {message}")
    } else {
        format!("{file}:{line}:{column}: {message}")
    }
}

/// Read a chart from `content`, the text of `file`. Errors are formatted
/// `file:line:column: message`, every one the reader found.
pub fn load(file: &str, content: &str, checks: Checks) -> Result<Loaded, Vec<String>> {
    let surface = Surface::detect(Path::new(file), content);
    match surface {
        Surface::Scxml => {
            let opts = ReadOptions::new()
                .with_datamodels(&ANY)
                .with_resolver(&DeferResolver);
            match vertep_scxml::read(content, &opts) {
                Ok(p) => {
                    let lost = p
                        .losses
                        .items
                        .iter()
                        .filter(|l| {
                            !matches!(
                                l.kind,
                                LossKind::ForeignExtension | LossKind::UnsupportedByDatamodel
                            )
                        })
                        .map(|l| located(file, l.span.line, l.span.column, &l.detail))
                        .collect();
                    Ok(Loaded {
                        chart: p.chart,
                        surface,
                        lost,
                    })
                }
                Err(e) => Err(e
                    .errors()
                    .iter()
                    .map(|e| located(file, e.span.line, e.span.column, &e.message))
                    .chain(match &e {
                        vertep_scxml::ReadError::Lossy(l) => l
                            .items
                            .iter()
                            .map(|l| located(file, l.span.line, l.span.column, &l.detail))
                            .collect::<Vec<_>>(),
                        _ => Vec::new(),
                    })
                    .collect()),
            }
        }
        Surface::Yaml => {
            let mut opts = vertep_yaml::ReadOptions::default().with_file(file);
            opts.typecheck = checks == Checks::Types;
            vertep_yaml::read(content, &opts)
                .map(|chart| Loaded {
                    chart,
                    surface,
                    lost: Vec::new(),
                })
                .map_err(|e| e.diagnostics.iter().map(|d| d.to_string()).collect())
        }
        rdf => {
            let Some(format) = rdf.rdf() else {
                unreachable!("every other surface is RDF")
            };
            vertep_rdf::read_chart(content.as_bytes(), format)
                .map(|r| Loaded {
                    chart: r.chart,
                    surface,
                    lost: Vec::new(),
                })
                .map_err(|e| vec![format!("{file}: {e}")])
        }
    }
}

/// Write `chart` in `surface`, or list what the surface cannot express.
pub fn write(chart: &Chart, surface: Surface) -> Result<String, Vec<String>> {
    match surface {
        Surface::Scxml => vertep_scxml::write(chart).map_err(|e| e.problems),
        Surface::Yaml => vertep_yaml::write(chart).map_err(|e| vec![e.to_string()]),
        rdf => {
            let Some(format) = rdf.rdf() else {
                unreachable!("every other surface is RDF")
            };
            Ok(vertep_rdf::write_chart(&vertep_rdf::project(chart), format))
        }
    }
}
