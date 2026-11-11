//! The graph store on a remote SPARQL 1.1 Query and Update endpoint.

use std::sync::Arc;

use oxigraph::io::{RdfFormat, RdfParser};
use oxrdf::{NamedNode, Quad, Triple};
use sparesults::{QueryResultsFormat, QueryResultsParser, SliceQueryResultsParserOutput};
use vertep_store::sparql::{clear_graph, delete_data, insert_data};
use vertep_store::{
    GraphCapabilities, GraphStore, QueryResults, RdfCapability, Solutions, StoreError,
};

use crate::http::{HttpClient, HttpRequest, HttpResponse, StdHttpClient};

const ACCEPT_QUERY: &str = "application/sparql-results+json, application/sparql-results+xml;q=0.9, \
     application/n-triples;q=0.9, text/turtle;q=0.8, application/rdf+xml;q=0.7";

/// A [`GraphStore`] on a remote SPARQL 1.1 endpoint: quads are written with SPARQL Update
/// (`INSERT DATA`, `DELETE DATA`, `CLEAR SILENT GRAPH`), queries are sent with SPARQL Query, both
/// as `POST` requests of the SPARQL 1.1 Protocol.
///
/// The endpoint's RDF version cannot be discovered portably, so it is declared by the
/// application ([`SparqlEndpoint::with_rdf`]); the default is RDF 1.1, which makes annotations
/// use the `rdf:Statement` fallback that every endpoint understands.
// @tg: implements:: [[openspec:storage#SPARQL Graph Store Adapter]]
#[derive(Clone)]
pub struct SparqlEndpoint {
    query_url: String,
    update_url: String,
    rdf: RdfCapability,
    client: Arc<dyn HttpClient>,
}

impl std::fmt::Debug for SparqlEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SparqlEndpoint")
            .field("query_url", &self.query_url)
            .field("update_url", &self.update_url)
            .field("rdf", &self.rdf)
            .finish_non_exhaustive()
    }
}

impl SparqlEndpoint {
    /// An endpoint answering queries and updates at `url`, declaring RDF 1.1, with the built-in
    /// plain-HTTP client.
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        SparqlEndpoint {
            query_url: url.clone(),
            update_url: url,
            rdf: RdfCapability::Rdf11,
            client: Arc::new(StdHttpClient::default()),
        }
    }

    /// This endpoint with a separate update URL.
    pub fn with_update_url(mut self, url: impl Into<String>) -> Self {
        self.update_url = url.into();
        self
    }

    /// This endpoint declaring `rdf` (whether it supports RDF 1.2 triple terms).
    pub fn with_rdf(mut self, rdf: RdfCapability) -> Self {
        self.rdf = rdf;
        self
    }

    /// This endpoint sending its requests through `client`.
    pub fn with_client(mut self, client: Arc<dyn HttpClient>) -> Self {
        self.client = client;
        self
    }

    fn post(
        &self,
        url: &str,
        content_type: &str,
        accept: &str,
        body: &str,
    ) -> Result<HttpResponse, StoreError> {
        let response = self.client.post(&HttpRequest {
            url,
            content_type,
            accept,
            body: body.as_bytes(),
        })?;
        match response.status {
            200..=299 => Ok(response),
            status => {
                let text = String::from_utf8_lossy(&response.body);
                let msg = format!("{url} answered {status}: {}", text.trim());
                Err(match status {
                    400 => StoreError::Invalid(msg),
                    408 | 429 | 502 | 503 | 504 => StoreError::Unavailable(msg),
                    _ => StoreError::Backend(msg),
                })
            }
        }
    }

    fn update(&self, text: &str) -> Result<(), StoreError> {
        self.post(&self.update_url, "application/sparql-update", "*/*", text)
            .map(|_| ())
    }
}

fn parse_results(response: &HttpResponse) -> Result<QueryResults, StoreError> {
    let media = response.content_type.as_deref().unwrap_or("");
    if let Some(format) = QueryResultsFormat::from_media_type(media) {
        return match QueryResultsParser::from_format(format)
            .for_slice(&response.body)
            .map_err(StoreError::backend)?
        {
            SliceQueryResultsParserOutput::Boolean(b) => Ok(QueryResults::Boolean(b)),
            SliceQueryResultsParserOutput::Solutions(parser) => {
                let variables: Vec<String> = parser
                    .variables()
                    .iter()
                    .map(|v| v.as_str().to_owned())
                    .collect();
                let mut rows = Vec::new();
                for s in parser {
                    rows.push(s.map_err(StoreError::backend)?.values().to_vec());
                }
                Ok(QueryResults::Solutions(Solutions { variables, rows }))
            }
        };
    }
    if let Some(format) = RdfFormat::from_media_type(media) {
        let mut triples = Vec::new();
        for q in RdfParser::from_format(format).for_slice(&response.body) {
            let q = q.map_err(StoreError::backend)?;
            triples.push(Triple::new(q.subject, q.predicate, q.object));
        }
        return Ok(QueryResults::Graph(triples));
    }
    Err(StoreError::Backend(format!(
        "unexpected SPARQL response media type {media:?}"
    )))
}

impl GraphStore for SparqlEndpoint {
    fn capabilities(&self) -> GraphCapabilities {
        GraphCapabilities::plain(self.rdf)
    }

    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        if quads.is_empty() {
            return Ok(());
        }
        if self.rdf == RdfCapability::Rdf11
            && quads
                .iter()
                .any(|q| matches!(q.object, oxrdf::Term::Triple(_)))
        {
            return Err(StoreError::Unsupported(
                "triple terms on an endpoint declared RDF 1.1".into(),
            ));
        }
        self.update(&insert_data(quads))
    }

    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        if quads.is_empty() {
            return Ok(());
        }
        self.update(&delete_data(quads)?)
    }

    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        self.update(&clear_graph(graph))
    }

    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        let response = self.post(
            &self.query_url,
            "application/sparql-query",
            ACCEPT_QUERY,
            sparql,
        )?;
        parse_results(&response)
    }
}
