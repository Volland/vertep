//! IRIs of the `vt-id:` identity module and the federation constants derived from them.

use oxrdf::NamedNode;

/// The `vt:` namespace (chart and runtime modules).
pub const VT: &str = "https://w3id.org/vertep/vt#";
/// The `vt-id:` namespace of the identity module.
pub const VT_ID: &str = "https://w3id.org/vertep/vt-id#";
/// The identity module's ontology IRI.
pub const IDENTITY_ONTOLOGY: &str = "https://w3id.org/vertep/vt/identity";
/// The Event I/O Processor type of the TSP transport (`_event.origintype`).
pub const TSP_PROCESSOR: &str = "https://w3id.org/vertep/vt-id#TSPEventProcessor";
/// The Event I/O Processor type of the DIDComm transport (`_event.origintype`).
pub const DIDCOMM_PROCESSOR: &str = "https://w3id.org/vertep/vt-id#DIDCommEventProcessor";
/// The JSON-LD context IRI of the relay envelope.
pub const RELAY_CONTEXT: &str = "https://w3id.org/vertep/contexts/relay-v1.jsonld";
/// The DIDComm message type that carries a relay envelope.
pub const DIDCOMM_EVENT_TYPE: &str = "https://w3id.org/vertep/protocols/event/1.0/event";
/// The verifiable-credential type of a grant credential.
pub const GRANT_CREDENTIAL_TYPE: &str = "VertepGrantCredential";
/// The JSON-LD context IRI of grant credentials.
pub const GRANT_CONTEXT: &str = "https://w3id.org/vertep/contexts/grant-v1.jsonld";
/// `rdf:type`.
pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
/// `xsd:boolean`.
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
/// `xsd:dateTime`.
pub const XSD_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";

/// A `vt-id:` term.
pub fn vt_id(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{VT_ID}{local}"))
}

/// A `vt:` term.
pub fn vt(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{VT}{local}"))
}

/// `rdf:type`.
pub fn rdf_type() -> NamedNode {
    NamedNode::new_unchecked(RDF_TYPE)
}
