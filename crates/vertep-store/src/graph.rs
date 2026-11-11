//! The graph store contract: RDF quads in, SPARQL out.

use oxrdf::{NamedNode, Quad, Term, Triple};

use crate::error::StoreError;

/// Which RDF version a graph store speaks for statements about statements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RdfCapability {
    /// RDF 1.2: triple terms and reifiers (`rdf:reifies`) can be written and queried.
    Rdf12,
    /// RDF 1.1 only: annotations use classic `rdf:Statement` reification
    /// ([`crate::reification`]).
    Rdf11,
}

/// What a graph store declares about itself, read before writing anything that depends on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCapabilities {
    /// RDF 1.2 triple terms, or the RDF 1.1 fallback.
    pub rdf: RdfCapability,
    /// The default graph of a query is the union of all graphs (true on tiramemsu), instead of
    /// only the statements outside named graphs (SPARQL's default). Portable queries use
    /// `GRAPH` and do not depend on it.
    pub union_default_graph: bool,
    /// IRI prefixes of predicates the store writes for its own bookkeeping (graph memberships,
    /// derivation links). Generic readers, such as annotation decoding, skip them.
    pub reserved_namespaces: Vec<String>,
}

impl GraphCapabilities {
    /// Capabilities of a plain SPARQL dataset with the given RDF version.
    pub fn plain(rdf: RdfCapability) -> Self {
        GraphCapabilities {
            rdf,
            union_default_graph: false,
            reserved_namespaces: Vec::new(),
        }
    }

    /// True when `predicate` is in one of the reserved namespaces.
    pub fn is_reserved(&self, predicate: &str) -> bool {
        self.reserved_namespaces
            .iter()
            .any(|ns| predicate.starts_with(ns.as_str()))
    }
}

/// A table of SPARQL solutions: variable names in projection order and rows of optional terms
/// (`None` is unbound).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Solutions {
    /// The variable names, without `?`.
    pub variables: Vec<String>,
    /// The rows, each parallel to `variables`.
    pub rows: Vec<Vec<Option<Term>>>,
}

impl Solutions {
    /// The column of variable `var`.
    pub fn column(&self, var: &str) -> Option<usize> {
        self.variables.iter().position(|v| v == var)
    }

    /// The value of `var` in row `row`, if bound.
    pub fn get(&self, row: usize, var: &str) -> Option<&Term> {
        let c = self.column(var)?;
        self.rows.get(row)?.get(c)?.as_ref()
    }

    /// Every bound value of `var`, in row order.
    pub fn values(&self, var: &str) -> Vec<Term> {
        let Some(c) = self.column(var) else {
            return Vec::new();
        };
        self.rows
            .iter()
            .filter_map(|r| r.get(c).cloned().flatten())
            .collect()
    }

    /// The number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The result of a SPARQL query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryResults {
    /// A `SELECT` result.
    Solutions(Solutions),
    /// An `ASK` answer.
    Boolean(bool),
    /// A `CONSTRUCT` or `DESCRIBE` graph.
    Graph(Vec<Triple>),
}

impl QueryResults {
    /// The solutions of a `SELECT`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] for another result form.
    pub fn into_solutions(self) -> Result<Solutions, StoreError> {
        match self {
            QueryResults::Solutions(s) => Ok(s),
            _ => Err(StoreError::invalid("expected SELECT solutions")),
        }
    }

    /// The answer of an `ASK`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] for another result form.
    pub fn into_boolean(self) -> Result<bool, StoreError> {
        match self {
            QueryResults::Boolean(b) => Ok(b),
            _ => Err(StoreError::invalid("expected an ASK answer")),
        }
    }

    /// The triples of a `CONSTRUCT`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] for another result form.
    pub fn into_graph(self) -> Result<Vec<Triple>, StoreError> {
        match self {
            QueryResults::Graph(g) => Ok(g),
            _ => Err(StoreError::invalid("expected a CONSTRUCT graph")),
        }
    }
}

/// A graph store: accepts RDF quads and answers SPARQL 1.1 queries, and declares whether it
/// supports RDF 1.2 triple terms.
///
/// Each call is one request: a store need not offer compare-and-set or transactions across
/// requests, which is why the journal is a separate contract. Writes of quads without blank
/// nodes are idempotent: inserting a quad twice leaves the store as inserting it once.
// @tg: implements:: [[openspec:storage#Graph Store Contract]]
pub trait GraphStore: Send + Sync {
    /// What the store declares about itself.
    fn capabilities(&self) -> GraphCapabilities;

    /// Whether the store supports RDF 1.2 triple terms.
    fn rdf_capability(&self) -> RdfCapability {
        self.capabilities().rdf
    }

    /// Adds `quads`.
    ///
    /// # Errors
    ///
    /// [`StoreError::Unavailable`] when the store cannot be reached, [`StoreError::Invalid`]
    /// for a quad it cannot hold, [`StoreError::Backend`] otherwise.
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError>;

    /// Removes `quads` (absent ones are ignored). Blank nodes cannot be named for removal.
    ///
    /// # Errors
    ///
    /// As [`GraphStore::insert_quads`].
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError>;

    /// Removes every statement of the named graph `graph`.
    ///
    /// # Errors
    ///
    /// As [`GraphStore::insert_quads`].
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError>;

    /// Runs a SPARQL 1.1 query (`SELECT`, `ASK`, `CONSTRUCT`, `DESCRIBE`).
    ///
    /// # Errors
    ///
    /// [`StoreError::Invalid`] for invalid query text, and the errors of
    /// [`GraphStore::insert_quads`].
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError>;
}

impl<G: GraphStore + ?Sized> GraphStore for std::sync::Arc<G> {
    fn capabilities(&self) -> GraphCapabilities {
        (**self).capabilities()
    }
    fn insert_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        (**self).insert_quads(quads)
    }
    fn remove_quads(&self, quads: &[Quad]) -> Result<(), StoreError> {
        (**self).remove_quads(quads)
    }
    fn clear_graph(&self, graph: &NamedNode) -> Result<(), StoreError> {
        (**self).clear_graph(graph)
    }
    fn query(&self, sparql: &str) -> Result<QueryResults, StoreError> {
        (**self).query(sparql)
    }
}
