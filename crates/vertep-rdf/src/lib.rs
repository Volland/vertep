//! RDF 1.2 projection, `vt:` chart ontology, SHACL shapes and canonical hashing for Vertep
//! charts.
//!
//! The chart IR is the hub; RDF is a lossless projection of it ([`project()`]), read back only
//! through the SHACL chart shapes ([`reconstruct()`]). A chart is named by the SHA-256 of the
//! RDFC-1.0 canonical N-Quads of its projection ([`chart_version`]), and its node IRIs are
//! minted under that version IRI. See `tg.md/views/rdf.md` for the architecture.
//!
//! @tg: implements:: [[decisions#Modular vt ontology on RDF 1.2]]
//! @tg: implements:: [[decisions#Content-addressed chart versions]] {confidence: partial, note: "hashing and version IRIs; pinning lives in vertep-runtime"}

pub mod canon;
pub mod hash;
pub mod io;
pub mod iri;
pub mod project;
pub mod reconstruct;
pub mod shacl;
pub mod shapes;
pub mod vocab;

pub use hash::{canonical_nquads, chart_hash, chart_version, graph_version};
pub use io::{RdfFormat, ReadError, read_chart, write_chart};
pub use iri::{ChartVersion, PLACEHOLDER_BASE};
pub use project::{ProjectedChart, project, project_with_base};
pub use reconstruct::{DecodeError, ReconstructError, Reconstructed, reconstruct};
pub use shacl::{ValidationReport, ValidationResult};
pub use shapes::{chart_ontology, chart_shapes, validate};
pub use vocab::NodeKind;
