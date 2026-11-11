//! `did:key`: the document is derived from the key itself, offline.

use crate::did::{Did, DidDocument, VerificationMethod};
use crate::keys::{KeyType, PublicKey};

use super::{DidResolver, Resolution, ResolutionError};

/// The `did:key` resolver (Ed25519 and X25519 multikeys). An Ed25519 key also gets the derived
/// X25519 key-agreement method, as the method specification describes.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyResolver;

/// The `did:key` DID of a public key.
pub fn did_key(key: &PublicKey) -> String {
    format!("did:key:{}", key.multikey())
}

/// The document a `did:key` (or `did:peer:0`) identifier with the multikey `mk` describes,
/// under the subject `did`.
///
/// # Errors
///
/// [`ResolutionError::InvalidDid`] when `mk` is not a supported multikey.
pub fn key_document(did: &str, mk: &str) -> Result<DidDocument, ResolutionError> {
    let key = PublicKey::from_multikey(mk)
        .map_err(|e| ResolutionError::InvalidDid(format!("{did}: {e}")))?;
    let mut doc = DidDocument::new(did);
    match key.key_type {
        KeyType::Ed25519 => {
            let vm_id = format!("{did}#{mk}");
            doc.verification_method
                .push(VerificationMethod::multikey(&vm_id, did, key));
            doc.authentication.push(vm_id.clone());
            doc.assertion_method.push(vm_id);
            let x = key
                .to_x25519()
                .map_err(|e| ResolutionError::InvalidDid(format!("{did}: {e}")))?;
            let x_id = format!("{did}#{}", x.multikey());
            doc.verification_method
                .push(VerificationMethod::multikey(&x_id, did, x));
            doc.key_agreement.push(x_id);
        }
        KeyType::X25519 => {
            let x_id = format!("{did}#{mk}");
            doc.verification_method
                .push(VerificationMethod::multikey(&x_id, did, key));
            doc.key_agreement.push(x_id);
        }
    }
    Ok(doc)
}

impl DidResolver for KeyResolver {
    // @tg: implements:: [[openspec:did-identity#Supported DID Methods]] {confidence: partial, note: "did:key"}
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        if did.method() != "key" {
            return Err(ResolutionError::MethodNotSupported(did.method().to_owned()));
        }
        Ok(Resolution::of(key_document(
            did.as_str(),
            did.method_specific_id(),
        )?))
    }
}
