//! Protocol offers and discovery.
//!
//! An [`Offer`] is a statement signed by a DID: "I play this role of this protocol", with an
//! optional expiry and an optional list of initiators it accepts. It is separate from the
//! protocol document, so it never changes the document's identity. A DID document lists the
//! protocols it offers in a service entry of type `VertepProtocol`; a peer fetches the document
//! by identity from anywhere and checks the hash itself.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::envelope::{SignError, Signer, Verifier, VerifyError};

/// The service type in a DID document.
pub const SERVICE_TYPE: &str = "VertepProtocol";

/// What an offer's signature covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferBody {
    /// The protocol identity.
    pub protocol: String,
    /// The role offered.
    pub role: String,
    /// The offering DID.
    pub did: String,
    /// When the offer lapses, in milliseconds since the epoch.
    #[serde(default, rename = "expiresMs", skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    /// The DIDs this agent accepts as initiators; anyone when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiators: Option<Vec<String>>,
}

/// A signed offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    /// The signed body.
    pub body: OfferBody,
    /// The verification method that signed it.
    pub key: String,
    /// The Ed25519 signature over the JCS bytes of the body, base64url without padding.
    pub signature: String,
}

/// One protocol in a DID document's service entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceProtocol {
    /// The protocol identity.
    pub protocol: String,
    /// The roles the DID plays.
    pub roles: Vec<String>,
}

impl Offer {
    /// Signs an offer.
    ///
    /// # Errors
    ///
    /// [`SignError`] when the signer refuses or signs for another DID.
    // @tg: implements:: [[openspec:agent-protocols#Protocol Offers]]
    pub fn sign(body: OfferBody, signer: &dyn Signer) -> Result<Self, SignError> {
        if signer.did() != body.did {
            return Err(SignError(format!(
                "signer is {}, offer is by {}",
                signer.did(),
                body.did
            )));
        }
        let bytes = serde_json_canonicalizer::to_vec(&body).unwrap_or_default();
        let signature = URL_SAFE_NO_PAD.encode(signer.sign(&bytes)?);
        Ok(Self {
            body,
            key: signer.key_id().to_owned(),
            signature,
        })
    }

    /// Checks the signature against the offering DID.
    ///
    /// # Errors
    ///
    /// [`VerifyError`] when it does not verify.
    // @tg: implements:: [[openspec:agent-protocols#Protocol Offers]]
    pub fn verify(&self, verifier: &dyn Verifier) -> Result<(), VerifyError> {
        let sig = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|e| VerifyError(e.to_string()))?;
        let bytes = serde_json_canonicalizer::to_vec(&self.body).unwrap_or_default();
        verifier.verify(&self.body.did, &self.key, &bytes, &sig)
    }

    /// Whether the offer has not lapsed at `now_ms`.
    #[must_use]
    pub fn is_valid_at(&self, now_ms: u64) -> bool {
        self.body.expires_ms.is_none_or(|e| now_ms < e)
    }

    /// Whether the offer accepts `initiator`.
    #[must_use]
    pub fn accepts_initiator(&self, initiator: &str) -> bool {
        self.body
            .initiators
            .as_ref()
            .is_none_or(|l| l.iter().any(|d| d == initiator))
    }
}

/// The `VertepProtocol` service entry of a DID document.
// @tg: implements:: [[openspec:agent-protocols#Protocol Discovery]]
#[must_use]
pub fn service_entry(did: &str, protocols: &[ServiceProtocol]) -> Value {
    json!({
        "id": format!("{did}#kolyada"),
        "type": SERVICE_TYPE,
        "serviceEndpoint": protocols,
    })
}

/// The protocols a DID document offers, read from its `VertepProtocol` service entries.
// @tg: implements:: [[openspec:agent-protocols#Protocol Discovery]]
#[must_use]
pub fn offered_protocols(did_document: &Value) -> Vec<ServiceProtocol> {
    let Some(services) = did_document.get("service").and_then(Value::as_array) else {
        return Vec::new();
    };
    services
        .iter()
        .filter(|s| s.get("type").and_then(Value::as_str) == Some(SERVICE_TYPE))
        .filter_map(|s| s.get("serviceEndpoint").cloned())
        .filter_map(|e| serde_json::from_value::<Vec<ServiceProtocol>>(e).ok())
        .flatten()
        .collect()
}

/// Checks that a document fetched by identity is the one that was listed: its hash is the id.
///
/// # Errors
///
/// A message naming the identity that was expected and the one that was found.
// @tg: implements:: [[openspec:agent-protocols#Protocol Discovery]]
pub fn verify_fetched(listed_id: &str, document: &[u8]) -> Result<crate::Protocol, String> {
    let p = crate::Protocol::parse(document).map_err(|e| e.to_string())?;
    if p.id().as_str() == listed_id {
        Ok(p)
    } else {
        Err(format!("expected {listed_id}, the document is {}", p.id()))
    }
}
