//! RDFC-1.0 canonicalization and canonical N-Quads.
//!
//! Blank node labelling is OxRDF's RDFC-1.0 implementation (the `rdfc-10` feature of `oxrdf`),
//! checked here against the W3C RDFC-1.0 test suite (`tests/rdfc_suite.rs`). This module adds
//! what the standard asks of a caller: the canonical N-Quads serialization (RDF 1.2 N-Quads
//! canonical form, lines sorted in code point order) and a guard against poisoned datasets.
//!
//! Chart projections contain no blank nodes (design D7), so for them canonicalization reduces
//! to serializing and sorting; the general path exists so that the hash is the standard one.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use oxrdf::dataset::{CanonicalizationAlgorithm, CanonicalizationHashAlgorithm};
use oxrdf::{
    Dataset, Graph, GraphNameRef, LiteralRef, NamedOrBlankNodeRef, QuadRef, TermRef, TripleRef,
};

use crate::vocab;

/// The hash function RDFC-1.0 is parametrized with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// SHA-256, the default and the one chart hashing uses.
    #[default]
    Sha256,
    /// SHA-384.
    Sha384,
}

/// The largest number of blank nodes related to one blank node in the same way (same
/// position, predicate and first-degree hash) that canonicalization accepts. Hash N-Degree Quads
/// explores every permutation of such a group, at every level of its recursion, so a larger
/// group is treated as a poisoned dataset (RDFC-1.0 section 4.1, "Dataset poisoning") and
/// refused instead of attempted. Chart projections contain no blank nodes at all.
pub const DEFAULT_MAX_RELATED_GROUP: usize = 6;

/// Canonicalization was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonError {
    /// A blank node has more indistinguishable related blank nodes than allowed.
    PoisonedDataset {
        /// The size of the largest group of indistinguishable related blank nodes.
        group: usize,
        /// The limit in force.
        limit: usize,
    },
}

impl std::fmt::Display for CanonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CanonError::PoisonedDataset { group, limit } => write!(
                f,
                "refusing to canonicalize: a blank node has {group} indistinguishable related \
                 blank nodes (limit {limit})"
            ),
        }
    }
}

impl std::error::Error for CanonError {}

/// The canonical form of a dataset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalForm {
    /// Canonical N-Quads: one line per quad, each ending in `\n`, sorted in code point order.
    pub nquads: String,
    /// The issued identifiers map: input blank node label to canonical label (`c14n<n>`).
    pub issued: BTreeMap<String, String>,
}

/// Canonicalize a dataset with RDFC-1.0, refusing datasets in which a blank node has more than
/// `max_related_group` indistinguishable related blank nodes.
pub fn canonicalize_with(
    dataset: &Dataset,
    algorithm: HashAlgorithm,
    max_related_group: usize,
) -> Result<CanonicalForm, CanonError> {
    let group = largest_related_group(dataset);
    if group > max_related_group {
        return Err(CanonError::PoisonedDataset {
            group,
            limit: max_related_group,
        });
    }
    let hash_algorithm = match algorithm {
        HashAlgorithm::Sha256 => CanonicalizationHashAlgorithm::Sha256,
        HashAlgorithm::Sha384 => CanonicalizationHashAlgorithm::Sha384,
    };
    let map =
        dataset.canonicalize_blank_nodes(CanonicalizationAlgorithm::Rdfc10 { hash_algorithm });
    let labels: HashMap<String, String> = map
        .iter()
        .map(|(from, to)| (from.as_str().to_owned(), to.as_str().to_owned()))
        .collect();
    let mut lines: Vec<String> = dataset
        .iter()
        .map(|q| {
            let mut line = String::new();
            write_quad(&mut line, q, &|b, out| {
                out.push_str(labels.get(b).map_or(b, String::as_str));
            });
            line
        })
        .collect();
    lines.sort_unstable();
    lines.dedup();
    let issued = labels.into_iter().collect();
    Ok(CanonicalForm {
        nquads: lines.concat(),
        issued,
    })
}

/// Canonicalize a dataset with RDFC-1.0 and SHA-256 under the default poisoning limit.
pub fn canonicalize(dataset: &Dataset) -> Result<CanonicalForm, CanonError> {
    canonicalize_with(dataset, HashAlgorithm::Sha256, DEFAULT_MAX_RELATED_GROUP)
}

/// The canonical N-Quads of a graph, as the default graph of a dataset.
pub fn canonical_nquads_of_graph(graph: &Graph) -> Result<String, CanonError> {
    let mut ds = Dataset::new();
    for t in graph.iter() {
        ds.insert(t.in_graph(GraphNameRef::DefaultGraph));
    }
    Ok(canonicalize(&ds)?.nquads)
}

/// The size of the largest set of blank nodes related to one blank node by the same position
/// and predicate and sharing a first-degree signature (the RDFC-1.0 input to the first-degree
/// hash): the groups whose permutations Hash N-Degree Quads explores.
fn largest_related_group(dataset: &Dataset) -> usize {
    let mut quads_of: HashMap<&str, Vec<QuadRef<'_>>> = HashMap::new();
    for q in dataset.iter() {
        let mut seen = Vec::new();
        quad_blank_nodes(q, &mut seen);
        seen.sort_unstable();
        seen.dedup();
        for b in seen {
            quads_of.entry(b).or_default().push(q);
        }
    }
    let signature: HashMap<&str, String> = quads_of
        .iter()
        .map(|(b, quads)| {
            let mut lines: Vec<String> = quads
                .iter()
                .map(|q| {
                    let mut line = String::new();
                    write_quad(&mut line, *q, &|x, out| {
                        out.push_str(if x == *b { "a" } else { "z" });
                    });
                    line
                })
                .collect();
            lines.sort_unstable();
            (*b, lines.concat())
        })
        .collect();
    let mut largest = 0;
    for (b, quads) in &quads_of {
        let mut groups: HashMap<(char, &str, &str), Vec<&str>> = HashMap::new();
        for q in quads {
            let mut related: Vec<(char, &str)> = Vec::new();
            if let NamedOrBlankNodeRef::BlankNode(s) = q.subject {
                related.push(('s', s.as_str()));
            }
            let mut objects = Vec::new();
            term_blank_nodes(q.object, &mut objects);
            related.extend(objects.into_iter().map(|o| ('o', o)));
            if let GraphNameRef::BlankNode(g) = q.graph_name {
                related.push(('g', g.as_str()));
            }
            for (position, r) in related {
                if r == *b {
                    continue;
                }
                let predicate = if position == 'g' {
                    ""
                } else {
                    q.predicate.as_str()
                };
                let members = groups
                    .entry((position, predicate, signature[r].as_str()))
                    .or_default();
                if !members.contains(&r) {
                    members.push(r);
                }
            }
        }
        largest = largest.max(groups.values().map(Vec::len).max().unwrap_or(0));
    }
    largest
}

fn quad_blank_nodes<'a>(q: QuadRef<'a>, out: &mut Vec<&'a str>) {
    if let NamedOrBlankNodeRef::BlankNode(b) = q.subject {
        out.push(b.as_str());
    }
    term_blank_nodes(q.object, out);
    if let GraphNameRef::BlankNode(b) = q.graph_name {
        out.push(b.as_str());
    }
}

fn term_blank_nodes<'a>(t: TermRef<'a>, out: &mut Vec<&'a str>) {
    match t {
        TermRef::BlankNode(b) => out.push(b.as_str()),
        TermRef::Triple(tr) => {
            if let NamedOrBlankNodeRef::BlankNode(b) = tr.subject.as_ref() {
                out.push(b.as_str());
            }
            term_blank_nodes(tr.object.as_ref(), out);
        }
        _ => {}
    }
}

/// Write one quad as a canonical N-Quads line, relabelling blank nodes with `label`.
pub fn write_quad(out: &mut String, q: QuadRef<'_>, label: Label<'_>) {
    write_subject(out, q.subject, label);
    out.push(' ');
    write_iri(out, q.predicate.as_str());
    out.push(' ');
    write_term(out, q.object, label);
    match q.graph_name {
        GraphNameRef::DefaultGraph => {}
        GraphNameRef::NamedNode(n) => {
            out.push(' ');
            write_iri(out, n.as_str());
        }
        GraphNameRef::BlankNode(b) => {
            out.push(' ');
            write_blank(out, b.as_str(), label);
        }
    }
    out.push_str(" .\n");
}

fn write_iri(out: &mut String, iri: &str) {
    out.push('<');
    out.push_str(iri);
    out.push('>');
}

fn write_blank(out: &mut String, id: &str, label: Label<'_>) {
    out.push_str("_:");
    label(id, out);
}

/// Writes the output label of a blank node given its input label.
pub type Label<'f> = &'f dyn Fn(&str, &mut String);

fn write_subject(out: &mut String, s: NamedOrBlankNodeRef<'_>, label: Label<'_>) {
    match s {
        NamedOrBlankNodeRef::NamedNode(n) => write_iri(out, n.as_str()),
        NamedOrBlankNodeRef::BlankNode(b) => write_blank(out, b.as_str(), label),
    }
}

fn write_triple(out: &mut String, t: TripleRef<'_>, label: Label<'_>) {
    out.push_str("<<( ");
    write_subject(out, t.subject, label);
    out.push(' ');
    write_iri(out, t.predicate.as_str());
    out.push(' ');
    write_term(out, t.object, label);
    out.push_str(" )>>");
}

fn write_term(out: &mut String, t: TermRef<'_>, label: Label<'_>) {
    match t {
        TermRef::NamedNode(n) => write_iri(out, n.as_str()),
        TermRef::BlankNode(b) => write_blank(out, b.as_str(), label),
        TermRef::Literal(l) => write_literal(out, l),
        TermRef::Triple(tr) => write_triple(out, tr.as_ref(), label),
    }
}

fn write_literal(out: &mut String, l: LiteralRef<'_>) {
    out.push('"');
    for c in l.value().chars() {
        match c {
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{C}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\0'..='\u{1F}' | '\u{7F}' => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    if let Some(lang) = l.language() {
        out.push('@');
        out.push_str(lang);
        if let Some(dir) = l.direction() {
            let _ = write!(out, "--{dir}");
        }
    } else if l.datatype().as_str() != vocab::XSD_STRING {
        out.push_str("^^");
        write_iri(out, l.datatype().as_str());
    }
}
