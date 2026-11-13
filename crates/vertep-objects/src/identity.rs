//! The address of a durable object: a DID whose document can change without changing the DID.

use core::fmt;

use oxrdf::NamedNode;
use vertep_federation::Did;

/// The DID methods a durable object may use: methods whose documents can be updated in place,
/// so custodians, endpoints and keys change while the address stays.
pub const MUTABLE_METHODS: [&str; 2] = ["webvh", "web"];

/// Why an address cannot name a durable object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    /// Not a DID.
    Syntax(String),
    /// A DID method whose document cannot change without changing the DID.
    ImmutableMethod(String),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityError::Syntax(m) => write!(f, "not a DID: {m}"),
            IdentityError::ImmutableMethod(m) => write!(
                f,
                "did:{m} cannot address a durable object: its document cannot change custodians \
                 or endpoints without changing the address (use did:webvh)"
            ),
        }
    }
}

impl std::error::Error for IdentityError {}

/// The address of a durable object.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DurableObjectId {
    did: Did,
}

impl DurableObjectId {
    /// The durable object addressed by `did`.
    ///
    /// # Errors
    ///
    /// [`IdentityError::ImmutableMethod`] for `did:key`, `did:peer` and every other method not
    /// in [`MUTABLE_METHODS`]; [`IdentityError::Syntax`] when `did` is not a DID.
    // @tg: implements:: [[openspec:durable-objects#Durable Object Identity]]
    pub fn new(did: &str) -> Result<Self, IdentityError> {
        let did = Did::parse(did).map_err(|e| IdentityError::Syntax(e.0))?;
        if !MUTABLE_METHODS.contains(&did.method()) {
            return Err(IdentityError::ImmutableMethod(did.method().to_owned()));
        }
        Ok(DurableObjectId { did })
    }

    /// The DID.
    pub fn did(&self) -> &Did {
        &self.did
    }

    /// The DID as a string.
    pub fn as_str(&self) -> &str {
        self.did.as_str()
    }

    /// The address as an IRI, the actor address of its journal.
    pub fn iri(&self) -> NamedNode {
        NamedNode::new_unchecked(self.did.as_str())
    }
}

impl fmt::Display for DurableObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.did.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvh_accepted() {
        let id = DurableObjectId::new("did:webvh:QmHash:example.org:objects:support").unwrap();
        assert_eq!(id.iri().as_str(), id.as_str());
    }

    // @tg: verifies:: [[openspec:durable-objects#Durable Object Identity#Immutable DID method refused]] {level: unit}
    #[test]
    fn immutable_did_method_refused() {
        let err = DurableObjectId::new("did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK")
            .unwrap_err();
        assert_eq!(err, IdentityError::ImmutableMethod("key".into()));
        assert!(err.to_string().contains("did:key"));
        assert!(matches!(
            DurableObjectId::new("did:peer:2.Ez6LS").unwrap_err(),
            IdentityError::ImmutableMethod(_)
        ));
    }
}
