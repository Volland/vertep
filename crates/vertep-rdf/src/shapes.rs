//! The published chart ontology and shapes, and validation of chart graphs against the shapes.

use std::sync::OnceLock;

use oxrdf::Graph;

use crate::io::{RdfFormat, read_graph};
use crate::shacl::{Shapes, ValidationReport};

/// The chart module of the `vt:` ontology, in Turtle.
// @tg: implements:: [[openspec:chart-rdf#Chart Ontology Module]]
pub const CHART_ONTOLOGY_TTL: &str = include_str!("../ontology/vt-chart.ttl");

/// The SHACL Core shapes of the chart module, in Turtle.
pub const CHART_SHAPES_TTL: &str = include_str!("../shapes/vt-chart-shapes.ttl");

/// The chart ontology module as a graph.
pub fn chart_ontology() -> Graph {
    match read_graph(CHART_ONTOLOGY_TTL.as_bytes(), RdfFormat::Turtle) {
        Ok(g) => g,
        // The file is part of the crate and parsed by the test suite.
        Err(e) => unreachable!("the bundled chart ontology parses: {e}"),
    }
}

/// The compiled chart shapes.
pub fn chart_shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let graph = match read_graph(CHART_SHAPES_TTL.as_bytes(), RdfFormat::Turtle) {
            Ok(g) => g,
            Err(e) => unreachable!("the bundled chart shapes parse: {e}"),
        };
        match Shapes::from_graph(&graph) {
            Ok(s) => s,
            Err(e) => unreachable!("the bundled chart shapes compile: {e}"),
        }
    })
}

/// Validate a graph against the chart shapes.
// @tg: implements:: [[openspec:chart-rdf#SHACL Shapes For Charts]]
pub fn validate(graph: &Graph) -> ValidationReport {
    chart_shapes().validate(graph)
}
