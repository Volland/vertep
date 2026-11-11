//! The `vt:` runtime ontology module and its SHACL shapes.
//!
//! The module itself is `vertep-rdf/ontology/vt-runtime.ttl` (next to the chart module); the
//! shapes ship with this crate. [`validate`] checks any runtime record (a journal entry as read
//! back from a store, a snapshot, a timer, a dead letter, a mailbox row, a current-state
//! graph) against them. The runtime's own queries use only `vt:` terms; [`rdfs_closure`] exists
//! for tests and tools that want to see the PROV-O view an RDFS-aware store would answer.

use std::collections::HashSet;
use std::sync::OnceLock;

use oxrdf::vocab::rdf;
use oxrdf::{Graph, NamedNode, NamedOrBlankNode, Term, TermRef, Triple};
use vertep_rdf::io::{RdfFormat, read_graph};
use vertep_rdf::shacl::{Shapes, ValidationReport};
use vertep_store::Entry;

/// The runtime module of the `vt:` ontology, in Turtle.
pub const RUNTIME_ONTOLOGY_TTL: &str = include_str!("../../vertep-rdf/ontology/vt-runtime.ttl");

/// The SHACL shapes of the runtime module, in Turtle.
pub const RUNTIME_SHAPES_TTL: &str = include_str!("../shapes/vt-runtime-shapes.ttl");

const RDFS_SUBCLASS: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_SUBPROPERTY: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";

/// The runtime ontology module as a graph: classes for actors (virtual, invoked, stateless
/// worker), addresses, mailboxes, activations, journal entries, macrosteps, snapshots, timers,
/// migrations and dead letters, aligned with PROV-O by subclass and subproperty statements.
// @tg: implements:: [[openspec:runtime-ontology#Runtime Vocabulary]]
// @tg: implements:: [[openspec:runtime-ontology#PROV-O Alignment Without Dependency]]
pub fn runtime_ontology() -> Graph {
    read_graph(RUNTIME_ONTOLOGY_TTL.as_bytes(), RdfFormat::Turtle).unwrap_or_default()
}

/// The compiled runtime shapes.
pub fn runtime_shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let g = read_graph(RUNTIME_SHAPES_TTL.as_bytes(), RdfFormat::Turtle)
            .unwrap_or_else(|e| panic!("the bundled runtime shapes parse: {e:?}"));
        Shapes::from_graph(&g)
            .unwrap_or_else(|e| panic!("the bundled runtime shapes compile: {e:?}"))
    })
}

/// Validates runtime records against the runtime shapes.
// @tg: implements:: [[openspec:runtime-ontology#Runtime SHACL Shapes]]
pub fn validate(data: &Graph) -> ValidationReport {
    runtime_shapes().validate(data)
}

/// Validates one journal entry, as read back from a journal store.
pub fn validate_entry(entry: &Entry) -> ValidationReport {
    validate(entry.triples())
}

/// `data` closed under the RDFS subclass and subproperty rules (rdfs5, rdfs7, rdfs9, rdfs11)
/// of `ontology`: what an RDFS-aware store infers. The runtime never needs this.
pub fn rdfs_closure(data: &Graph, ontology: &Graph) -> Graph {
    let sub = |p: &str| -> Vec<(NamedNode, NamedNode)> {
        let p = NamedNode::new_unchecked(p);
        ontology
            .triples_for_predicate(&p)
            .filter_map(|t| match (t.subject, t.object) {
                (oxrdf::NamedOrBlankNodeRef::NamedNode(a), TermRef::NamedNode(b)) => {
                    Some((a.into_owned(), b.into_owned()))
                }
                _ => None,
            })
            .collect()
    };
    let closure = |pairs: Vec<(NamedNode, NamedNode)>| {
        let mut set: HashSet<(NamedNode, NamedNode)> = pairs.into_iter().collect();
        loop {
            let mut added = Vec::new();
            for (a, b) in &set {
                for (c, d) in &set {
                    if b == c && !set.contains(&(a.clone(), d.clone())) {
                        added.push((a.clone(), d.clone()));
                    }
                }
            }
            if added.is_empty() {
                break set;
            }
            set.extend(added);
        }
    };
    let classes = closure(sub(RDFS_SUBCLASS));
    let props = closure(sub(RDFS_SUBPROPERTY));
    let mut out = data.clone();
    let mut extra: Vec<Triple> = Vec::new();
    for t in data.iter() {
        let s: NamedOrBlankNode = t.subject.into_owned();
        let p = t.predicate.into_owned();
        let o: Term = t.object.into_owned();
        for (a, b) in &props {
            if *a == p {
                extra.push(Triple::new(s.clone(), b.clone(), o.clone()));
            }
        }
    }
    for t in &extra {
        out.insert(t);
    }
    let typed: Vec<(NamedOrBlankNode, NamedNode)> = out
        .triples_for_predicate(rdf::TYPE)
        .filter_map(|t| match t.object {
            TermRef::NamedNode(c) => Some((t.subject.into_owned(), c.into_owned())),
            _ => None,
        })
        .collect();
    for (s, c) in typed {
        for (a, b) in &classes {
            if *a == c {
                out.insert(&Triple::new(s.clone(), rdf::TYPE, b.clone()));
            }
        }
    }
    out
}
