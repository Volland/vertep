//! Graph stores on Oxigraph and on any SPARQL 1.1 endpoint.
//!
//! [`OxigraphStore`] runs an embedded Oxigraph store (in memory unless given an opened store),
//! and [`SparqlEndpoint`] talks the SPARQL 1.1 Protocol to a remote endpoint: quads are written
//! by SPARQL Update, queries answered by SPARQL Query. Both implement only
//! [`vertep_store::GraphStore`]: SPARQL Update has no compare-and-set, so neither may hold a
//! journal. Journals stay in a journal store such as tiramemsu, and the
//! [`vertep_store::Projector`] copies them here.
//!
//! @tg: implements:: [[decisions#Two storage traits with tiramemsu as default]] {confidence: partial, note: "graph store adapter only"}

mod embedded;
mod http;
mod remote;

pub use embedded::OxigraphStore;
pub use http::{HttpClient, HttpRequest, HttpResponse, StdHttpClient};
pub use remote::SparqlEndpoint;

/// The Oxigraph crate this adapter is built on.
pub use oxigraph;
