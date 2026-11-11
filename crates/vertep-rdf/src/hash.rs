//! The canonical chart hash and the chart version IRI derived from it.
//!
//! The hash of a chart is the SHA-256 of the RDFC-1.0 canonical N-Quads of its projection
//! minted under [`crate::iri::PLACEHOLDER_BASE`]. Minting under a fixed base is what breaks the
//! circularity between node IRIs (which live under the version IRI) and the version IRI (which
//! is the hash of the graph containing them): the hashed graph and the published graph differ
//! only by renaming the base, so either determines the other.
//!
//! Spans are not projected and annotations are not part of a chart, so the hash depends only on
//! the chart's structure.

use oxrdf::Graph;
use sha2::{Digest, Sha256};
use vertep_core::ir::Chart;

use crate::canon;
use crate::iri::ChartVersion;
use crate::project::project_unversioned;
use crate::reconstruct::{ReconstructError, reconstruct};

/// The RDFC-1.0 canonical N-Quads of a chart's projection under the placeholder base.
pub fn canonical_nquads(chart: &Chart) -> String {
    let graph = project_unversioned(chart);
    match canon::canonical_nquads_of_graph(&graph) {
        Ok(nquads) => nquads,
        // A projection has no blank nodes, so the poisoning guard cannot trigger.
        Err(e) => unreachable!("chart projection has no blank nodes: {e}"),
    }
}

/// The SHA-256 of the chart's canonical N-Quads.
// @tg: implements:: [[openspec:chart-rdf#Canonical Chart Hash]]
pub fn chart_hash(chart: &Chart) -> [u8; 32] {
    Sha256::digest(canonical_nquads(chart).as_bytes()).into()
}

/// The version IRI `urn:vertep:chart:sha256:<hex>` of a chart.
pub fn chart_version(chart: &Chart) -> ChartVersion {
    ChartVersion::from_digest(&chart_hash(chart))
}

/// The version of the chart a graph holds: the graph is validated and reconstructed, so triples
/// outside the projection (annotations) do not take part, and then hashed.
pub fn graph_version(graph: &Graph) -> Result<ChartVersion, ReconstructError> {
    Ok(chart_version(&reconstruct(graph)?.chart))
}
