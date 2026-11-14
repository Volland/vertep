//! The one-way bridge from protocols and offers to RDF.
//!
//! A protocol's graph is the expanded graph its identity is computed from
//! ([`crate::Protocol::graph`]). An offer becomes three triples under the same vocabulary, so a
//! graph store holding both answers "which DIDs offer this role of this protocol" with SPARQL.
//! Nothing flows back: the identity is never computed from anything but the document.

use oxrdf::{BlankNode, Graph, Literal, NamedNode, Triple};

use crate::offer::Offer;

/// The `vk:` namespace.
pub const VK: &str = "https://w3id.org/vertep/kolyada#";

fn vk(term: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{VK}{term}"))
}

/// Adds an offer to `graph`: the DID `vk:offers` a node that names a `vk:protocol` and a
/// `vk:role`.
///
/// # Errors
///
/// A message when the DID or the protocol identity is not a valid IRI.
// @tg: implements:: [[openspec:agent-protocols#RDF Bridge]]
pub fn add_offer(graph: &mut Graph, offer: &Offer) -> Result<(), String> {
    let did = NamedNode::new(&offer.body.did).map_err(|e| e.to_string())?;
    let protocol = NamedNode::new(&offer.body.protocol).map_err(|e| e.to_string())?;
    let node = BlankNode::default();
    graph.insert(&Triple::new(did, vk("offers"), node.clone()));
    graph.insert(&Triple::new(node.clone(), vk("protocol"), protocol));
    graph.insert(&Triple::new(
        node,
        vk("role"),
        Literal::new_simple_literal(&offer.body.role),
    ));
    Ok(())
}

/// Adds the graph of a protocol to `graph`.
// @tg: implements:: [[openspec:agent-protocols#RDF Bridge]]
pub fn add_protocol(graph: &mut Graph, protocol: &crate::Protocol) {
    for t in protocol.graph().iter() {
        graph.insert(t);
    }
}
