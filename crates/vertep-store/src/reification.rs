//! Annotations on statements, written as RDF 1.2 reifiers or, on stores that only speak
//! RDF 1.1, as classic `rdf:Statement` reification, and read back as the same annotation.
//!
//! The encoding is selected by the graph store's declared [`RdfCapability`]:
//!
//! | Capability | Quads written for `s p o` annotated with `k v` |
//! |---|---|
//! | RDF 1.2 | `s p o`, `r rdf:reifies <<( s p o )>>`, `r k v` |
//! | RDF 1.1 | `s p o`, `r rdf:type rdf:Statement`, `r rdf:subject s`, `r rdf:predicate p`, `r rdf:object o`, `r k v` |
//!
//! In both encodings the annotated statement itself is asserted, as RDF 1.2 annotation syntax
//! (`s p o {| k v |}`) does.

use std::collections::BTreeMap;

use oxrdf::vocab::rdf;
use oxrdf::{
    BlankNode, Dataset, GraphName, GraphNameRef, NamedNode, NamedOrBlankNode, Quad, Term, TermRef,
    Triple,
};

use crate::error::StoreError;
use crate::graph::{GraphStore, RdfCapability};

/// `rdf:reifies` (RDF 1.2).
pub const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

/// An annotation: properties about one asserted statement in one graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    /// The graph holding the statement and its annotation.
    pub graph: GraphName,
    /// The annotated statement.
    pub triple: Triple,
    /// The resource standing for the statement (the reifier). A store may rename it: tiramemsu
    /// resolves every reifier of a statement to the statement's own id.
    pub reifier: NamedOrBlankNode,
    /// The annotation's properties, such as a confidence or the journal entry that caused it.
    pub properties: Vec<(NamedNode, Term)>,
}

impl Annotation {
    /// An annotation of `triple` in `graph` with a fresh blank node reifier.
    pub fn new(graph: impl Into<GraphName>, triple: Triple) -> Self {
        Annotation {
            graph: graph.into(),
            triple,
            reifier: BlankNode::default().into(),
            properties: Vec::new(),
        }
    }

    /// This annotation with reifier `r`.
    pub fn with_reifier(mut self, r: impl Into<NamedOrBlankNode>) -> Self {
        self.reifier = r.into();
        self
    }

    /// This annotation with one more property.
    pub fn with(mut self, key: NamedNode, value: impl Into<Term>) -> Self {
        self.properties.push((key, value.into()));
        self
    }

    /// True when both annotate the same statement in the same graph with the same set of
    /// properties, whatever their reifiers.
    pub fn same_content(&self, other: &Annotation) -> bool {
        let mut a = self.properties.clone();
        let mut b = other.properties.clone();
        a.sort_by_key(|(k, v)| (k.to_string(), v.to_string()));
        a.dedup();
        b.sort_by_key(|(k, v)| (k.to_string(), v.to_string()));
        b.dedup();
        self.graph == other.graph && self.triple == other.triple && a == b
    }
}

/// The quads encoding `annotation` for a store with capability `rdf`.
// @tg: implements:: [[openspec:storage#RDF 1.1 Reification Fallback]]
pub fn encode(annotation: &Annotation, rdf: RdfCapability) -> Vec<Quad> {
    let g = annotation.graph.clone();
    let r = annotation.reifier.clone();
    let t = &annotation.triple;
    let mut quads = vec![t.clone().in_graph(g.clone())];
    match rdf {
        RdfCapability::Rdf12 => quads.push(Quad::new(
            r.clone(),
            NamedNode::new_unchecked(RDF_REIFIES),
            Term::Triple(Box::new(t.clone())),
            g.clone(),
        )),
        RdfCapability::Rdf11 => {
            quads.push(Quad::new(r.clone(), rdf::TYPE, rdf::STATEMENT, g.clone()));
            quads.push(Quad::new(
                r.clone(),
                rdf::SUBJECT,
                t.subject.clone(),
                g.clone(),
            ));
            quads.push(Quad::new(
                r.clone(),
                rdf::PREDICATE,
                t.predicate.clone(),
                g.clone(),
            ));
            quads.push(Quad::new(
                r.clone(),
                rdf::OBJECT,
                t.object.clone(),
                g.clone(),
            ));
        }
    }
    for (k, v) in &annotation.properties {
        quads.push(Quad::new(r.clone(), k.clone(), v.clone(), g.clone()));
    }
    quads
}

/// Writes `annotations` to `store` in the encoding its declared capability selects.
///
/// # Errors
///
/// The errors of [`GraphStore::insert_quads`].
pub fn write_annotations(
    store: &dyn GraphStore,
    annotations: &[Annotation],
) -> Result<(), StoreError> {
    let rdf = store.rdf_capability();
    let quads: Vec<Quad> = annotations.iter().flat_map(|a| encode(a, rdf)).collect();
    store.insert_quads(&quads)
}

fn structural(rdf: RdfCapability, key: &NamedNode, value: &Term) -> bool {
    match rdf {
        RdfCapability::Rdf12 => key.as_str() == RDF_REIFIES,
        RdfCapability::Rdf11 => {
            key.as_ref() == rdf::SUBJECT
                || key.as_ref() == rdf::PREDICATE
                || key.as_ref() == rdf::OBJECT
                || (key.as_ref() == rdf::TYPE && value.as_ref() == TermRef::from(rdf::STATEMENT))
        }
    }
}

/// Reads the annotations of `triple` in `graph` from `store`, in the encoding its declared
/// capability selects. Annotations by different reifiers come back separately.
///
/// # Errors
///
/// The errors of [`GraphStore::query`].
pub fn read_annotations(
    store: &dyn GraphStore,
    graph: &GraphName,
    triple: &Triple,
) -> Result<Vec<Annotation>, StoreError> {
    let caps = store.capabilities();
    let (s, p, o) = (&triple.subject, &triple.predicate, &triple.object);
    let body = match caps.rdf {
        RdfCapability::Rdf12 => format!("{s} {p} {o} ~ ?r {{| ?k ?v |}} ."),
        RdfCapability::Rdf11 => format!(
            "{s} {p} {o} . ?r a <{st}> ; <{rs}> {s} ; <{rp}> {p} ; <{ro}> {o} ; ?k ?v .",
            st = rdf::STATEMENT.as_str(),
            rs = rdf::SUBJECT.as_str(),
            rp = rdf::PREDICATE.as_str(),
            ro = rdf::OBJECT.as_str(),
        ),
    };
    let query = match graph {
        GraphName::NamedNode(g) => format!("SELECT ?r ?k ?v WHERE {{ GRAPH {g} {{ {body} }} }}"),
        GraphName::DefaultGraph => format!("SELECT ?r ?k ?v WHERE {{ {body} }}"),
        GraphName::BlankNode(_) => {
            return Err(StoreError::invalid("annotations in a blank-node graph"));
        }
    };
    let solutions = store.query(&query)?.into_solutions()?;
    let mut by_reifier: BTreeMap<String, Annotation> = BTreeMap::new();
    for i in 0..solutions.len() {
        let (Some(r), Some(Term::NamedNode(k)), Some(v)) = (
            solutions.get(i, "r"),
            solutions.get(i, "k"),
            solutions.get(i, "v"),
        ) else {
            continue;
        };
        let reifier = match r {
            Term::NamedNode(n) => NamedOrBlankNode::from(n.clone()),
            Term::BlankNode(b) => NamedOrBlankNode::from(b.clone()),
            _ => continue,
        };
        let a = by_reifier
            .entry(r.to_string())
            .or_insert_with(|| Annotation {
                graph: graph.clone(),
                triple: triple.clone(),
                reifier,
                properties: Vec::new(),
            });
        if structural(caps.rdf, k, v) || caps.is_reserved(k.as_str()) {
            continue;
        }
        let prop = (k.clone(), v.clone());
        if !a.properties.contains(&prop) {
            a.properties.push(prop);
        }
    }
    Ok(by_reifier
        .into_values()
        .filter(|a| !a.properties.is_empty())
        .collect())
}

/// Decodes every annotation in `dataset`, in both encodings: RDF 1.2 reifiers and RDF 1.1
/// `rdf:Statement` resources. Only annotations of statements asserted in the same graph are
/// returned.
pub fn decode(dataset: &Dataset) -> Vec<Annotation> {
    let mut out = Vec::new();
    // RDF 1.2: r rdf:reifies <<( s p o )>>
    let reifies = NamedNode::new_unchecked(RDF_REIFIES);
    for q in dataset.quads_for_predicate(&reifies) {
        let TermRef::Triple(t) = q.object else {
            continue;
        };
        let triple = t.as_ref().into_owned();
        let graph = q.graph_name.into_owned();
        if !dataset.contains(&triple.clone().in_graph(graph.clone())) {
            continue;
        }
        let reifier = q.subject.into_owned();
        let properties = properties_of(dataset, &reifier, &graph, RdfCapability::Rdf12);
        out.push(Annotation {
            graph,
            triple,
            reifier,
            properties,
        });
    }
    // RDF 1.1: r a rdf:Statement ; rdf:subject s ; rdf:predicate p ; rdf:object o
    for q in dataset.quads_for_object(rdf::STATEMENT) {
        if q.predicate != rdf::TYPE {
            continue;
        }
        let graph = q.graph_name.into_owned();
        let reifier = q.subject.into_owned();
        let one = |p: oxrdf::NamedNodeRef<'_>| -> Option<Term> {
            dataset
                .quads_for_subject(&reifier)
                .find(|x| x.predicate == p && x.graph_name == GraphNameRef::from(&graph))
                .map(|x| x.object.into_owned())
        };
        let (Some(s), Some(Term::NamedNode(p)), Some(o)) =
            (one(rdf::SUBJECT), one(rdf::PREDICATE), one(rdf::OBJECT))
        else {
            continue;
        };
        let s = match s {
            Term::NamedNode(n) => NamedOrBlankNode::from(n),
            Term::BlankNode(b) => NamedOrBlankNode::from(b),
            _ => continue,
        };
        let triple = Triple::new(s, p, o);
        if !dataset.contains(&triple.clone().in_graph(graph.clone())) {
            continue;
        }
        let properties = properties_of(dataset, &reifier, &graph, RdfCapability::Rdf11);
        out.push(Annotation {
            graph,
            triple,
            reifier,
            properties,
        });
    }
    out
}

fn properties_of(
    dataset: &Dataset,
    reifier: &NamedOrBlankNode,
    graph: &GraphName,
    rdf: RdfCapability,
) -> Vec<(NamedNode, Term)> {
    dataset
        .quads_for_subject(reifier)
        .filter(|q| q.graph_name == GraphNameRef::from(graph))
        .map(|q| (q.predicate.into_owned(), q.object.into_owned()))
        .filter(|(k, v)| !structural(rdf, k, v))
        .collect()
}
