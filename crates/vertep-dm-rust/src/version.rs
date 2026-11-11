//! The chart version IRI of a chart, as `vt hash` computes it.

use vertep_core::ir::Chart;

/// The chart version IRI (`urn:vertep:chart:sha256:<hex>`): the SHA-256 of the RDFC-1.0
/// canonical projection of the chart ([[openspec:chart-rdf#Canonical Chart Hash]]). The
/// generated `VERSION` constant is this function of the generated `CHART`.
// @tg: implements:: [[openspec:rust-dsl#Generated Typed API]] {confidence: partial, note: "computed once at first use, not in the compiler: proc macros do not go through RDF"}
pub fn chart_version_iri(chart: &Chart) -> String {
    vertep_rdf::chart_version(chart).as_str().to_string()
}
