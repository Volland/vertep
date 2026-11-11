//! The `vt-id:` identity module of the `vt:` ontology, its SHACL shapes, and the projection of
//! DID documents, DID actors, grants and inbound records into RDF.
//!
//! Projections hold public keys and key ids only. The runtime writes them into the actor's
//! current-state graph; the store's history keeps earlier document versions queryable after a
//! rotation.

use std::sync::OnceLock;

use oxrdf::{BlankNode, Graph, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sha2::{Digest, Sha256};
use vertep_rdf::io::{RdfFormat, read_graph};
use vertep_rdf::shacl::{Shapes, ValidationReport};

use crate::did::DidDocument;
use crate::grants::Grant;
use crate::origin::InboundRecord;
use crate::time::Timestamp;
use crate::vocab::{XSD_BOOLEAN, XSD_DATE_TIME, rdf_type, vt, vt_id};

/// The identity module, in Turtle.
pub const IDENTITY_ONTOLOGY_TTL: &str = include_str!("../ontology/vt-id.ttl");
/// The SHACL shapes of the identity module, in Turtle.
pub const IDENTITY_SHAPES_TTL: &str = include_str!("../shapes/vt-id-shapes.ttl");
/// The JSON-LD context of grant credentials, served at [`crate::vocab::GRANT_CONTEXT`].
pub const GRANT_CONTEXT_JSON: &str = include_str!("../ontology/grant-v1.jsonld");

/// A projection that could not name a node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionError(pub String);

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot project: {} is not an IRI", self.0)
    }
}

impl std::error::Error for ProjectionError {}

/// The identity module as a graph.
// @tg: implements:: [[openspec:did-identity#DID Documents In The Runtime Graph]]
pub fn identity_ontology() -> Graph {
    match read_graph(IDENTITY_ONTOLOGY_TTL.as_bytes(), RdfFormat::Turtle) {
        Ok(g) => g,
        Err(e) => unreachable!("the bundled identity module parses: {e}"),
    }
}

/// The compiled identity shapes.
pub fn identity_shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let g = match read_graph(IDENTITY_SHAPES_TTL.as_bytes(), RdfFormat::Turtle) {
            Ok(g) => g,
            Err(e) => unreachable!("the bundled identity shapes parse: {e}"),
        };
        match Shapes::from_graph(&g) {
            Ok(s) => s,
            Err(e) => unreachable!("the bundled identity shapes compile: {e}"),
        }
    })
}

/// Validates identity data against the identity shapes.
pub fn validate(data: &Graph) -> ValidationReport {
    identity_shapes().validate(data)
}

fn iri(s: &str) -> Result<NamedNode, ProjectionError> {
    NamedNode::new(s).map_err(|_| ProjectionError(s.to_owned()))
}

fn add(g: &mut Graph, s: impl Into<NamedOrBlankNode>, p: NamedNode, o: impl Into<Term>) {
    g.insert(&Triple::new(s, p, o));
}

fn string(s: &str) -> Literal {
    Literal::new_simple_literal(s)
}

fn date_time(t: Timestamp) -> Literal {
    Literal::new_typed_literal(t.to_rfc3339(), NamedNode::new_unchecked(XSD_DATE_TIME))
}

fn boolean(b: bool) -> Literal {
    Literal::new_typed_literal(
        if b { "true" } else { "false" },
        NamedNode::new_unchecked(XSD_BOOLEAN),
    )
}

/// Projects a DID document: the DID, its verification methods with their public keys, its
/// verification relationships and its services, and the document version when known.
///
/// # Errors
///
/// [`ProjectionError`] when an id is not an IRI.
// @tg: implements:: [[openspec:did-identity#DID Documents In The Runtime Graph]]
// @tg: implements:: [[openspec:did-identity#Key Management Interface]] {confidence: partial, note: "the graph receives public keys only"}
pub fn project_document(
    doc: &DidDocument,
    version_id: Option<&str>,
) -> Result<Graph, ProjectionError> {
    let mut g = Graph::new();
    let did = iri(&doc.id)?;
    for vm in &doc.verification_method {
        let v = iri(&vm.id)?;
        add(&mut g, did.clone(), vt_id("verificationMethod"), v.clone());
        add(&mut g, v.clone(), rdf_type(), vt_id("VerificationMethod"));
        add(&mut g, v.clone(), vt_id("controller"), iri(&vm.controller)?);
        add(
            &mut g,
            v.clone(),
            vt_id("publicKeyMultibase"),
            string(&vm.public_key.multikey()),
        );
        add(
            &mut g,
            v,
            vt_id("keyType"),
            string(vm.public_key.key_type.as_str()),
        );
    }
    for (rel, ids) in [
        ("authentication", &doc.authentication),
        ("assertionMethod", &doc.assertion_method),
        ("keyAgreement", &doc.key_agreement),
    ] {
        for id in ids {
            add(&mut g, did.clone(), vt_id(rel), iri(id)?);
        }
    }
    for svc in &doc.service {
        let s = iri(&svc.id)?;
        add(&mut g, did.clone(), vt_id("service"), s.clone());
        add(&mut g, s.clone(), rdf_type(), vt_id("Service"));
        for t in &svc.types {
            add(&mut g, s.clone(), vt_id("serviceType"), string(t));
        }
        for ep in &svc.endpoints {
            add(&mut g, s.clone(), vt_id("serviceEndpoint"), string(&ep.uri));
            for k in &ep.routing_keys {
                add(&mut g, s.clone(), vt_id("routingKey"), string(k));
            }
        }
    }
    if let Some(v) = version_id {
        add(&mut g, did, vt_id("documentVersion"), string(v));
    }
    Ok(g)
}

/// Projects a DID actor: its DID as a `vt-id:DidActor` (a `vt:Actor`) and its document.
///
/// # Errors
///
/// [`ProjectionError`] when an id is not an IRI.
pub fn project_actor(
    doc: &DidDocument,
    version_id: Option<&str>,
) -> Result<Graph, ProjectionError> {
    let mut g = project_document(doc, version_id)?;
    let did = iri(&doc.id)?;
    add(&mut g, did.clone(), rdf_type(), vt_id("DidActor"));
    add(&mut g, did, rdf_type(), vt("Actor"));
    Ok(g)
}

fn grant_node(grant: &Grant) -> Result<NamedOrBlankNode, ProjectionError> {
    Ok(match &grant.id {
        Some(id) => iri(id)?.into(),
        None => {
            let mut h = Sha256::new();
            for part in [&grant.grantee, &grant.target] {
                h.update(part.as_bytes());
                h.update([0]);
            }
            for e in &grant.events {
                h.update(e.as_bytes());
                h.update([0]);
            }
            h.update(grant.expires.unwrap_or(-1).to_be_bytes());
            let digest = h.finalize();
            let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
            BlankNode::new_unchecked(format!("grant{hex}")).into()
        }
    })
}

/// Projects grants and, when given, the actor that holds them.
///
/// # Errors
///
/// [`ProjectionError`] when a grantee, target, issuer or holder is not an IRI.
// @tg: implements:: [[openspec:did-identity#DID Documents In The Runtime Graph]]
pub fn project_grants(grants: &[Grant], holder: Option<&str>) -> Result<Graph, ProjectionError> {
    let mut g = Graph::new();
    for grant in grants {
        let n = grant_node(grant)?;
        add(&mut g, n.clone(), rdf_type(), vt_id("Grant"));
        add(&mut g, n.clone(), vt_id("grantee"), iri(&grant.grantee)?);
        add(&mut g, n.clone(), vt_id("target"), iri(&grant.target)?);
        for e in &grant.events {
            add(&mut g, n.clone(), vt_id("event"), string(e));
        }
        if let Some(exp) = grant.expires {
            add(
                &mut g,
                n.clone(),
                vt_id("expires"),
                date_time(Timestamp(exp)),
            );
        }
        if let Some(issuer) = &grant.issuer {
            add(&mut g, n.clone(), vt_id("issuer"), iri(issuer)?);
            add(&mut g, iri(issuer)?, vt_id("issuedGrant"), n.clone());
        }
        if let Some(h) = holder {
            add(&mut g, iri(h)?, vt_id("holdsGrant"), n);
        }
    }
    Ok(g)
}

/// The IRI of an inbound record: `<actor>/inbound/<hex SHA-256 of the message id>`, or of the
/// receipt instant when the message could not be opened.
pub fn inbound_record_iri(record: &InboundRecord) -> String {
    let key = match &record.message_id {
        Some(id) => format!("m:{id}"),
        None => format!("t:{}", record.at),
    };
    let hex: String = Sha256::digest(key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{}/inbound/{hex}", record.actor)
}

/// Projects an inbound record: receiver, message id, event, verified origin and flag, and the
/// acceptance or refusal with its reason. Auditors query these for every event an actor
/// accepted from a DID.
///
/// # Errors
///
/// [`ProjectionError`] when the actor or sender is not an IRI.
// @tg: implements:: [[openspec:did-identity#Journaled Verification Outcome]] {note: "the audit projection of the outcome"}
pub fn project_inbound(record: &InboundRecord) -> Result<Graph, ProjectionError> {
    let mut g = Graph::new();
    let n = iri(&inbound_record_iri(record))?;
    add(&mut g, n.clone(), rdf_type(), vt_id("InboundMessage"));
    add(&mut g, n.clone(), vt_id("receivedBy"), iri(&record.actor)?);
    add(
        &mut g,
        n.clone(),
        vt_id("receivedAt"),
        date_time(Timestamp(record.at)),
    );
    if let Some(id) = &record.message_id {
        add(&mut g, n.clone(), vt_id("messageId"), string(id));
    }
    if let Some(e) = &record.event {
        add(&mut g, n.clone(), vt("eventName"), string(e));
    }
    let verified = record.origin.as_ref().is_some_and(|o| o.verified);
    add(
        &mut g,
        n.clone(),
        vt_id("originVerified"),
        boolean(verified),
    );
    if let Some(o) = &record.origin {
        if let (true, Some(s)) = (o.verified, &o.sender) {
            add(&mut g, n.clone(), vt_id("verifiedOrigin"), iri(s)?);
        }
        if let Some(k) = &o.sender_key {
            add(&mut g, n.clone(), vt_id("senderKey"), iri(k)?);
        }
        add(
            &mut g,
            n.clone(),
            vt_id("ioProcessor"),
            iri(&o.io_processor)?,
        );
    }
    add(
        &mut g,
        n.clone(),
        vt_id("accepted"),
        boolean(record.refusal.is_none()),
    );
    if let Some(r) = &record.refusal {
        add(&mut g, n, vt_id("refusalReason"), string(&r.to_string()));
    }
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_shapes_load() {
        let g = identity_ontology();
        assert!(g.len() > 50);
        assert!(identity_shapes().len() >= 7);
        let iri = NamedNode::new_unchecked(crate::vocab::IDENTITY_ONTOLOGY);
        assert!(g.iter().any(|t| t.subject == iri.as_ref().into()));
    }
}
