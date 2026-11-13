//! DID documents for new `did:webvh` DIDs, built from Vertep identities.
//!
//! A new DID's identity is provisioned under the template DID (`did:webvh:{SCID}:...`), so its
//! verification method and service ids carry the placeholder that creation replaces; after
//! creation, [`bind`] rewrites the identity to the real DID.

use serde_json::Value;
use vertep_federation::{
    ActorIdentity, DidDocument, DidMethodChoice, IdentityConfig, KeyError, KeyManager, Service,
};

use crate::author::CreateOptions;
use crate::error::WebvhError;

/// Provisions an identity (an Ed25519 signing key and an X25519 key-agreement key in `keys`)
/// with `services` for a DID created with `options`.
///
/// # Errors
///
/// [`KeyError`] when the key manager cannot create the keys.
pub fn provision(
    keys: &dyn KeyManager,
    options: &CreateOptions,
    services: Vec<Service>,
) -> Result<ActorIdentity, KeyError> {
    ActorIdentity::provision(
        keys,
        &IdentityConfig::new(DidMethodChoice::Hosted {
            did: options.template_did(),
            services,
        }),
    )
}

/// The DID document of `identity`: its authentication, assertion and key-agreement methods
/// and its services.
///
/// # Errors
///
/// [`WebvhError::Keys`] when the key manager no longer holds the identity's keys.
// @tg: implements:: [[openspec:webvh-identity#Document From An Identity]]
pub fn identity_document(
    identity: &ActorIdentity,
    keys: &dyn KeyManager,
) -> Result<Value, WebvhError> {
    Ok(identity.document(keys)?.to_json())
}

/// Rewrites an identity provisioned under a template DID to the created DID.
pub fn bind(identity: &mut ActorIdentity, did: &str) {
    let template = identity.did.clone();
    let fix = |s: &str| s.replacen(&template, did, 1);
    identity.signing.kid = fix(&identity.signing.kid);
    identity.agreement.kid = fix(&identity.agreement.kid);
    for r in &mut identity.retired {
        r.key.kid = fix(&r.key.kid);
    }
    for s in &mut identity.services {
        s.id = fix(&s.id);
    }
    identity.did = did.to_owned();
}

/// `document` with the services of type `service_type` replaced by `with`.
///
/// # Errors
///
/// [`WebvhError::Malformed`] when `document` is not a DID document.
pub fn replace_service(
    document: &Value,
    service_type: &str,
    with: Service,
) -> Result<Value, WebvhError> {
    let mut doc =
        DidDocument::from_json(document).map_err(|e| WebvhError::Malformed(e.to_string()))?;
    doc.service.retain(|s| !s.has_type(service_type));
    doc.service.push(with);
    Ok(doc.to_json())
}

#[cfg(feature = "custody")]
mod custody {
    use serde_json::Value;
    use vertep_federation::{ActorIdentity, KeyManager, PublicKey, VerificationMethod};
    use vertep_objects::custody::CustodyConfig;
    use vertep_objects::custody::config::CUSTODY_SERVICE as CUSTODY_SERVICE_TYPE;

    use crate::error::WebvhError;
    use crate::signer::did_key;

    /// The DID document of a durable object: `identity`'s keys and services, the
    /// `VertepCustody` declaration of `custody`, and the controller's key, which signs custody
    /// reconfigurations.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Keys`] when the key manager no longer holds the identity's keys.
    pub fn object_document(
        identity: &ActorIdentity,
        keys: &dyn KeyManager,
        custody: &CustodyConfig,
        controller: &PublicKey,
    ) -> Result<Value, WebvhError> {
        let mut doc = identity.document(keys)?;
        doc.service.retain(|s| !s.has_type(CUSTODY_SERVICE_TYPE));
        doc.service.push(custody.to_service(&identity.did));
        doc.verification_method.push(VerificationMethod::multikey(
            format!("{}#controller", identity.did),
            did_key(controller),
            *controller,
        ));
        Ok(doc.to_json())
    }

    /// `document` declaring the custody configuration `custody` instead of its current one.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Malformed`] when `document` is not a DID document.
    pub fn with_custody(document: &Value, custody: &CustodyConfig) -> Result<Value, WebvhError> {
        let id = document
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| WebvhError::Malformed("the document has no id".into()))?;
        super::replace_service(document, CUSTODY_SERVICE_TYPE, custody.to_service(id))
    }
}

#[cfg(feature = "custody")]
pub use custody::{object_document, with_custody};
