//! The `vt:` terms the storage layer writes itself.
//!
//! These are the bookkeeping terms of the journal protocol: the type and stamps of a journal
//! entry, the actor's head, the fencing epoch per scope and the projector's position. They live
//! in the Vertep namespace ([`VT`]) so that entries use the runtime ontology's terms; the
//! runtime vocabulary module in `vertep-rdf` is expected to declare the same IRIs.

use oxrdf::NamedNode;

/// The Vertep namespace (equal to `vertep_core::ir::VT_NS`).
pub const VT: &str = "https://w3id.org/vertep/vt#";

/// The `rdf:type` IRI.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// `vt:JournalEntry`: the class of journal entries.
pub const JOURNAL_ENTRY: &str = "https://w3id.org/vertep/vt#JournalEntry";
/// `vt:actor`: from a journal entry to the address of the actor whose journal it is in.
pub const ACTOR: &str = "https://w3id.org/vertep/vt#actor";
/// `vt:position`: the journal position of an entry (`xsd:integer`, from 1, no gaps).
pub const POSITION: &str = "https://w3id.org/vertep/vt#position";
/// `vt:head`: the position of the last entry of an actor's journal (cardinality one).
pub const HEAD: &str = "https://w3id.org/vertep/vt#head";
/// `vt:fencingEpoch`: the highest fencing epoch accepted for a scope (cardinality one).
pub const FENCING_EPOCH: &str = "https://w3id.org/vertep/vt#fencingEpoch";
/// `vt:projectedPosition`: the last journal position of an actor written to a graph store.
pub const PROJECTED_POSITION: &str = "https://w3id.org/vertep/vt#projectedPosition";
/// The graph in which a [`crate::Projector`] records its positions.
pub const PROJECTION_GRAPH: &str = "https://w3id.org/vertep/vt#projection";

/// A [`NamedNode`] for one of the IRI constants of this module.
pub fn node(iri: &str) -> NamedNode {
    NamedNode::new_unchecked(iri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_are_in_the_vt_namespace() {
        for iri in [
            JOURNAL_ENTRY,
            ACTOR,
            POSITION,
            HEAD,
            FENCING_EPOCH,
            PROJECTED_POSITION,
            PROJECTION_GRAPH,
        ] {
            assert!(iri.starts_with(VT), "{iri}");
            assert!(NamedNode::new(iri).is_ok());
        }
    }
}
