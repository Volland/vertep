//! Reading and writing chart graphs as Turtle, N-Triples, N-Quads and TriG (RDF 1.2).
//!
//! In the dataset formats (N-Quads, TriG) a chart's triples are in the named graph whose name
//! is the chart's version IRI. Output is deterministic: triples are written in sorted order.

use core::fmt;

use oxrdf::{Dataset, Graph, GraphName, GraphNameRef, Quad, Triple};
use oxttl::{
    NQuadsParser, NQuadsSerializer, NTriplesParser, NTriplesSerializer, TriGParser, TriGSerializer,
    TurtleParser, TurtleSerializer,
};

use crate::project::ProjectedChart;
use crate::reconstruct::{ReconstructError, Reconstructed, reconstruct};
use crate::vocab;

/// An RDF 1.2 concrete syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RdfFormat {
    /// Turtle.
    Turtle,
    /// N-Triples.
    NTriples,
    /// N-Quads (a dataset format).
    NQuads,
    /// TriG (a dataset format).
    TriG,
}

impl RdfFormat {
    /// Every format.
    pub const ALL: [RdfFormat; 4] = [
        RdfFormat::Turtle,
        RdfFormat::NTriples,
        RdfFormat::NQuads,
        RdfFormat::TriG,
    ];

    /// Whether the format carries named graphs.
    pub fn is_dataset(self) -> bool {
        matches!(self, RdfFormat::NQuads | RdfFormat::TriG)
    }
}

/// A document could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// The document is not valid in its format.
    Syntax(String),
    /// A dataset does not hold exactly one graph with a `vt:Chart`.
    NoChartGraph(usize),
    /// The graph is not a valid chart projection.
    Reconstruct(ReconstructError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Syntax(e) => write!(f, "syntax error: {e}"),
            ReadError::NoChartGraph(n) => {
                write!(f, "expected one graph holding a vt:Chart, found {n}")
            }
            ReadError::Reconstruct(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {}

fn syntax(e: impl fmt::Display) -> ReadError {
    ReadError::Syntax(e.to_string())
}

fn sorted(graph: &Graph) -> Vec<Triple> {
    let mut ts: Vec<Triple> = graph.iter().map(|t| t.into_owned()).collect();
    ts.sort_by_cached_key(|t| t.to_string());
    ts
}

fn written<W>(r: std::io::Result<W>) -> W {
    match r {
        Ok(w) => w,
        // Every serializer here writes into a Vec, which cannot fail.
        Err(e) => unreachable!("writing to memory failed: {e}"),
    }
}

fn utf8(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => unreachable!("the serializers write UTF-8: {e}"),
    }
}

/// Write a graph's triples. Dataset formats put them in `graph_name`, or in the default graph
/// when it is `None`. `prefixes` are used by Turtle and TriG.
pub fn write_graph(
    graph: &Graph,
    format: RdfFormat,
    graph_name: Option<&GraphName>,
    prefixes: &[(&str, &str)],
) -> String {
    let triples = sorted(graph);
    let name = graph_name.cloned().unwrap_or(GraphName::DefaultGraph);
    let bytes = match format {
        RdfFormat::Turtle => {
            let mut s = TurtleSerializer::new();
            for (p, iri) in prefixes {
                s = s
                    .with_prefix(*p, *iri)
                    .unwrap_or_else(|e| unreachable!("prefix: {e}"));
            }
            let mut w = s.for_writer(Vec::new());
            for t in &triples {
                written(w.serialize_triple(t));
            }
            written(w.finish())
        }
        RdfFormat::NTriples => {
            let mut w = NTriplesSerializer::new().for_writer(Vec::new());
            for t in &triples {
                written(w.serialize_triple(t));
            }
            w.finish()
        }
        RdfFormat::NQuads => {
            let mut w = NQuadsSerializer::new().for_writer(Vec::new());
            for t in &triples {
                written(w.serialize_quad(&t.clone().in_graph(name.clone())));
            }
            w.finish()
        }
        RdfFormat::TriG => {
            let mut s = TriGSerializer::new();
            for (p, iri) in prefixes {
                s = s
                    .with_prefix(*p, *iri)
                    .unwrap_or_else(|e| unreachable!("prefix: {e}"));
            }
            let mut w = s.for_writer(Vec::new());
            for t in &triples {
                written(w.serialize_quad(&t.clone().in_graph(name.clone())));
            }
            written(w.finish())
        }
    };
    utf8(bytes)
}

/// Write a projected chart. In N-Quads and TriG its triples are in the named graph whose name
/// is its version IRI.
// @tg: implements:: [[openspec:chart-rdf#RDF Serialization Formats]]
pub fn write_chart(chart: &ProjectedChart, format: RdfFormat) -> String {
    let base = format!("{}#", chart.version);
    let prefixes = [
        ("vt", vocab::VT),
        ("xsd", "http://www.w3.org/2001/XMLSchema#"),
        ("c", base.as_str()),
    ];
    let name = GraphName::NamedNode(chart.version.named_node());
    write_graph(&chart.graph, format, Some(&name), &prefixes)
}

/// Read a dataset; a graph format is read into the default graph.
pub fn read_dataset(input: &[u8], format: RdfFormat) -> Result<Dataset, ReadError> {
    let mut ds = Dataset::new();
    match format {
        RdfFormat::Turtle => {
            for t in TurtleParser::new().for_slice(input) {
                ds.insert(&t.map_err(syntax)?.in_graph(GraphName::DefaultGraph));
            }
        }
        RdfFormat::NTriples => {
            for t in NTriplesParser::new().for_slice(input) {
                ds.insert(&t.map_err(syntax)?.in_graph(GraphName::DefaultGraph));
            }
        }
        RdfFormat::NQuads => {
            for q in NQuadsParser::new().for_slice(input) {
                ds.insert(&q.map_err(syntax)?);
            }
        }
        RdfFormat::TriG => {
            for q in TriGParser::new().for_slice(input) {
                ds.insert(&q.map_err(syntax)?);
            }
        }
    }
    Ok(ds)
}

/// Read a graph. For a dataset format, the triples of every graph are merged.
pub fn read_graph(input: &[u8], format: RdfFormat) -> Result<Graph, ReadError> {
    let ds = read_dataset(input, format)?;
    let mut g = Graph::new();
    for q in ds.iter() {
        g.insert(&Triple::new(
            q.subject.into_owned(),
            q.predicate.into_owned(),
            q.object.into_owned(),
        ));
    }
    Ok(g)
}

/// The graph of a dataset that holds the chart: the only graph (default or named) with a
/// `vt:Chart` node.
pub fn chart_graph(ds: &Dataset) -> Result<Graph, ReadError> {
    let rdf_type = oxrdf::NamedNodeRef::new_unchecked(vocab::RDF_TYPE);
    let mut names: Vec<GraphName> = ds
        .iter()
        .filter(|q| q.predicate == rdf_type && q.object == vocab::CHART.into())
        .map(|q| q.graph_name.into_owned())
        .collect();
    names.sort_by_cached_key(|n| n.to_string());
    names.dedup();
    let [name] = names.as_slice() else {
        return Err(ReadError::NoChartGraph(names.len()));
    };
    let mut g = Graph::new();
    for q in ds.quads_for_graph_name(GraphNameRef::from(name)) {
        g.insert(&Quad::from(q).into());
    }
    Ok(g)
}

/// Read a document holding one chart and reconstruct it.
pub fn read_chart(input: &[u8], format: RdfFormat) -> Result<Reconstructed, ReadError> {
    let ds = read_dataset(input, format)?;
    let graph = chart_graph(&ds)?;
    reconstruct(&graph).map_err(ReadError::Reconstruct)
}
