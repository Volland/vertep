//! SPARQL text for graph store writes, and conversion of evaluator results.
//!
//! Backends that speak SPARQL Update (a remote endpoint, tiramemsu) write quads as
//! `INSERT DATA` and `DELETE DATA` requests built here. Terms are written in their N-Triples
//! form, which is valid SPARQL 1.2 (triple terms as `<<( s p o )>>`).

use std::collections::BTreeMap;
use std::fmt::Write;

use oxrdf::{GraphName, NamedNode, NamedOrBlankNode, Quad, Term};

use crate::error::StoreError;
use crate::graph::{QueryResults, Solutions};

/// Groups quads by graph, keeping the order of first appearance within each graph.
fn by_graph(quads: &[Quad]) -> BTreeMap<String, (GraphName, Vec<&Quad>)> {
    let mut groups: BTreeMap<String, (GraphName, Vec<&Quad>)> = BTreeMap::new();
    for q in quads {
        groups
            .entry(q.graph_name.to_string())
            .or_insert_with(|| (q.graph_name.clone(), Vec::new()))
            .1
            .push(q);
    }
    groups
}

fn write_block(out: &mut String, quads: &[Quad]) {
    for (_, (g, qs)) in by_graph(quads) {
        let indent = match &g {
            GraphName::DefaultGraph => "  ",
            _ => {
                let _ = writeln!(out, "  GRAPH {g} {{");
                "    "
            }
        };
        for q in qs {
            let _ = writeln!(out, "{indent}{} {} {} .", q.subject, q.predicate, q.object);
        }
        if !matches!(g, GraphName::DefaultGraph) {
            out.push_str("  }\n");
        }
    }
}

/// `INSERT DATA { ... }` for `quads`.
pub fn insert_data(quads: &[Quad]) -> String {
    let mut out = String::from("INSERT DATA {\n");
    write_block(&mut out, quads);
    out.push('}');
    out
}

fn has_blank(q: &Quad) -> bool {
    matches!(q.subject, NamedOrBlankNode::BlankNode(_))
        || term_has_blank(&q.object)
        || matches!(q.graph_name, GraphName::BlankNode(_))
}

fn term_has_blank(t: &Term) -> bool {
    match t {
        Term::BlankNode(_) => true,
        Term::Triple(tr) => {
            matches!(tr.subject, NamedOrBlankNode::BlankNode(_)) || term_has_blank(&tr.object)
        }
        _ => false,
    }
}

/// `DELETE DATA { ... }` for `quads`.
///
/// # Errors
///
/// [`StoreError::Invalid`] when a quad contains a blank node, which `DELETE DATA` cannot name.
pub fn delete_data(quads: &[Quad]) -> Result<String, StoreError> {
    if quads.iter().any(has_blank) {
        return Err(StoreError::invalid(
            "blank nodes cannot be named for removal",
        ));
    }
    let mut out = String::from("DELETE DATA {\n");
    write_block(&mut out, quads);
    out.push('}');
    Ok(out)
}

/// `CLEAR SILENT GRAPH <graph>`.
pub fn clear_graph(graph: &NamedNode) -> String {
    format!("CLEAR SILENT GRAPH {graph}")
}

/// Collects a `spareval` result (what Oxigraph and the in-memory reference store return) into
/// [`QueryResults`].
///
/// # Errors
///
/// [`StoreError::Backend`] when evaluation fails while iterating.
pub fn collect_results(results: spareval::QueryResults<'_>) -> Result<QueryResults, StoreError> {
    match results {
        spareval::QueryResults::Boolean(b) => Ok(QueryResults::Boolean(b)),
        spareval::QueryResults::Solutions(iter) => {
            let variables = iter
                .variables()
                .iter()
                .map(|v| v.as_str().to_owned())
                .collect();
            let mut rows = Vec::new();
            for s in iter {
                let s = s.map_err(StoreError::backend)?;
                rows.push(s.values().to_vec());
            }
            Ok(QueryResults::Solutions(Solutions { variables, rows }))
        }
        spareval::QueryResults::Graph(iter) => {
            let mut triples = Vec::new();
            for t in iter {
                triples.push(t.map_err(StoreError::backend)?);
            }
            Ok(QueryResults::Graph(triples))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, Triple};

    fn n(s: &str) -> NamedNode {
        NamedNode::new(format!("urn:t:{s}")).unwrap()
    }

    #[test]
    fn insert_data_groups_by_graph() {
        let quads = vec![
            Quad::new(n("a"), n("p"), Literal::from("x\"y"), n("g")),
            Quad::new(n("a"), n("p"), n("b"), GraphName::DefaultGraph),
            Quad::new(
                n("r"),
                n("reifies"),
                Term::Triple(Box::new(Triple::new(n("a"), n("p"), n("b")))),
                n("g"),
            ),
        ];
        let text = insert_data(&quads);
        assert!(text.starts_with("INSERT DATA {"));
        assert!(text.contains("GRAPH <urn:t:g> {"));
        assert!(text.contains(r#"<urn:t:a> <urn:t:p> "x\"y" ."#));
        assert!(text.contains("<<( <urn:t:a> <urn:t:p> <urn:t:b> )>>"));
        spargebra::SparqlParser::new()
            .parse_update(&text)
            .expect("valid SPARQL 1.2 update");
    }

    #[test]
    fn delete_data_refuses_blank_nodes() {
        let q = Quad::new(BlankNode::default(), n("p"), n("o"), n("g"));
        assert!(delete_data(&[q]).is_err());
        let ok = delete_data(&[Quad::new(n("s"), n("p"), n("o"), n("g"))]).unwrap();
        spargebra::SparqlParser::new().parse_update(&ok).unwrap();
        spargebra::SparqlParser::new()
            .parse_update(&clear_graph(&n("g")))
            .unwrap();
    }
}
