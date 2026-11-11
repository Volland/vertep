//! Projections of the chart type language ([`vertep_core::types`]) to RDF, SHACL and JSON
//! Schema, and content-addressed type IRIs.
//!
//! - [`ident`] serializes named types to RDF and names each one by the SHA-256 of the
//!   RDFC-1.0 canonical N-Quads of its definition: `urn:vertep:type:sha256:<hex>`
//!   ([`TypeIri`], [`TypeIris`]).
//! - [`shapes`] projects types to SHACL Core shapes; a named type's node shape is its type IRI.
//!   [`value`] encodes runtime values in RDF so that the encoding of a value conforms to the
//!   shapes of a type exactly when the core checker accepts the value.
//! - [`json_schema`] projects types to JSON Schema 2020-12 documents with the same agreement.
//! - [`project_chart_types`] adds a chart's named types (definitions and shapes) to its RDF
//!   projection, so the chart version covers its types.
//!
//! The vocabulary is the type module of the `vt:` ontology, `ontology/vt-types.ttl`
//! ([`TYPES_ONTOLOGY_TTL`]).

pub mod ident;
pub mod json_schema;
pub mod shapes;
pub mod value;

use oxrdf::{Graph, NamedNode, NamedNodeRef, TripleRef};
use vertep_core::ir::Chart;
use vertep_core::types::chart::declarations;

pub use ident::{TYPE_PREFIX, TypeIri, TypeIriError, TypeIris};
pub use json_schema::{ChartSchemas, json_schema};
pub use shapes::{TypeShapes, validate_value};
pub use value::value_graph;

use crate::iri;
use crate::vocab as vt;

/// The type module of the `vt:` ontology, in Turtle.
pub const TYPES_ONTOLOGY_TTL: &str = include_str!("../../ontology/vt-types.ttl");

/// The type module of the `vt:` ontology as a graph.
pub fn types_ontology() -> Graph {
    match crate::io::read_graph(TYPES_ONTOLOGY_TTL.as_bytes(), crate::io::RdfFormat::Turtle) {
        Ok(g) => g,
        Err(e) => unreachable!("the bundled type ontology parses: {e}"),
    }
}

/// Add the chart's named types to its projection minted under `base`: a `vt:declaresType`
/// edge from the chart node to every named type whose IRI can be derived, and, under each
/// type IRI, the type's definition and its SHACL node shape. A chart without a parsable
/// `vt:types` block adds nothing, so untyped charts keep their M0 projection and version.
// @tg: implements:: [[openspec:type-system#Named Content-Addressed Types]]
// @tg: implements:: [[openspec:type-system#SHACL Projection]]
// @tg: implements:: [[openspec:type-system#Types Survive Every Surface]]
pub fn project_chart_types(graph: &mut Graph, chart: &Chart, base: &str) {
    let Some(Ok(decls)) = declarations(chart) else {
        return;
    };
    let iris = TypeIris::of(&decls);
    if iris.is_empty() {
        return;
    }
    let chart_node = NamedNode::new_unchecked(iri::node_iri(base, iri::CHART_FRAGMENT));
    for (_, t) in iris.iter() {
        graph.insert(TripleRef::new(&chart_node, vt::DECLARES_TYPE, t.as_ref()));
    }
    graph.extend(iris.definitions(&decls).iter());
    let mut shapes = TypeShapes::new(&decls, &iris);
    shapes.named_types();
    graph.extend(shapes.into_graph().iter());
}

/// `rdf:type`.
pub(crate) fn rdf_type() -> NamedNodeRef<'static> {
    NamedNodeRef::new_unchecked(vt::RDF_TYPE)
}
