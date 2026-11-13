//! The `vt:` terms of durable objects and custody.

use oxrdf::NamedNode;

/// `vt:custodyEpoch`: the custody epoch an entry was written in (`xsd:integer`).
pub const CUSTODY_EPOCH: &str = "https://w3id.org/vertep/vt#custodyEpoch";
/// `vt:writer`: the DID of the host that wrote and signed an entry.
pub const WRITER: &str = "https://w3id.org/vertep/vt#writer";
/// `vt:previousEntryHash`: the content hash of the preceding entry (hex SHA-256).
pub const PREVIOUS_ENTRY_HASH: &str = "https://w3id.org/vertep/vt#previousEntryHash";
/// `vt:entryHash`: the entry's own content hash (hex SHA-256).
pub const ENTRY_HASH: &str = "https://w3id.org/vertep/vt#entryHash";
/// `vt:entrySignature`: the writer's signature over the content hash (base64url).
pub const ENTRY_SIGNATURE: &str = "https://w3id.org/vertep/vt#entrySignature";

/// `vt:DurableObject`: the class of durable objects.
pub const DURABLE_OBJECT: &str = "https://w3id.org/vertep/vt#DurableObject";
/// `vt:CustodyEpoch`: the class of custody epochs.
pub const CUSTODY_EPOCH_CLASS: &str = "https://w3id.org/vertep/vt#CustodyEpoch";
/// `vt:epochOf`: from a custody epoch to its object.
pub const EPOCH_OF: &str = "https://w3id.org/vertep/vt#epochOf";
/// `vt:epochNumber`: the number of a custody epoch.
pub const EPOCH_NUMBER: &str = "https://w3id.org/vertep/vt#epochNumber";
/// `vt:epochHost`: the host that wrote in a custody epoch.
pub const EPOCH_HOST: &str = "https://w3id.org/vertep/vt#epochHost";
/// `vt:custodian`: a custodian of an epoch's custodian set.
pub const CUSTODIAN: &str = "https://w3id.org/vertep/vt#custodian";
/// `vt:promisedBy`: a custodian whose promise made a host the writer of an epoch.
pub const PROMISED_BY: &str = "https://w3id.org/vertep/vt#promisedBy";
/// `vt:firstPosition`: the first journal position written in an epoch.
pub const FIRST_POSITION: &str = "https://w3id.org/vertep/vt#firstPosition";
/// `vt:lastPosition`: the last journal position written in an epoch.
pub const LAST_POSITION: &str = "https://w3id.org/vertep/vt#lastPosition";
/// `vt:Equivocation`: two validly signed entries for one position and epoch.
pub const EQUIVOCATION: &str = "https://w3id.org/vertep/vt#Equivocation";
/// `vt:conflictingEntry`: an entry of an equivocation.
pub const CONFLICTING_ENTRY: &str = "https://w3id.org/vertep/vt#conflictingEntry";

/// A [`NamedNode`] for one of the IRI constants of this module.
pub fn node(iri: &str) -> NamedNode {
    NamedNode::new_unchecked(iri)
}

/// The Turtle of the durable-object and custody ontology module.
pub const CUSTODY_ONTOLOGY_TTL: &str = include_str!("../ontology/custody.ttl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ontology_parses_and_declares_the_terms() {
        let mut iris = std::collections::BTreeSet::new();
        for t in oxttl::TurtleParser::new().for_slice(CUSTODY_ONTOLOGY_TTL.as_bytes()) {
            let t = t.expect("valid Turtle");
            if let oxrdf::NamedOrBlankNode::NamedNode(n) = t.subject {
                iris.insert(n.into_string());
            }
        }
        for term in [
            CUSTODY_EPOCH,
            WRITER,
            PREVIOUS_ENTRY_HASH,
            ENTRY_HASH,
            ENTRY_SIGNATURE,
            DURABLE_OBJECT,
            CUSTODY_EPOCH_CLASS,
            EPOCH_OF,
            EPOCH_NUMBER,
            EPOCH_HOST,
            CUSTODIAN,
            PROMISED_BY,
            FIRST_POSITION,
            LAST_POSITION,
            EQUIVOCATION,
            CONFLICTING_ENTRY,
        ] {
            assert!(iris.contains(term), "{term} is not declared");
        }
        for m in [
            "prepare", "promise", "refuse", "append", "ack", "read", "entries",
        ] {
            assert!(iris.contains(&format!("https://w3id.org/vertep/custody/1.0#{m}")));
        }
    }
}
