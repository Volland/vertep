//! An Ed25519 [`Signer`] and [`Verifier`] over in-memory keys, for tests, vectors and examples.
//! Production code signs through the key manager of `vertep-federation`.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

use crate::envelope::{SignError, Signer, Verifier, VerifyError};

/// A key held in memory, named by a DID and a verification method id.
pub struct SoftSigner {
    did: String,
    key_id: String,
    key: SigningKey,
}

impl SoftSigner {
    /// A signer whose key is derived from `seed`, so tests and vectors are deterministic.
    #[must_use]
    pub fn from_seed(did: &str, seed: [u8; 32]) -> Self {
        Self {
            did: did.to_owned(),
            key_id: format!("{did}#key-1"),
            key: SigningKey::from_bytes(&seed),
        }
    }

    /// The public key bytes.
    #[must_use]
    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
}

impl Signer for SoftSigner {
    fn did(&self) -> &str {
        &self.did
    }

    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignError> {
        Ok(self.key.sign(data).to_bytes().to_vec())
    }
}

/// A set of public keys by `(did, key id)`.
#[derive(Default)]
pub struct StaticVerifier {
    keys: BTreeMap<(String, String), VerifyingKey>,
}

impl StaticVerifier {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the key of a signer.
    pub fn add(&mut self, signer: &SoftSigner) {
        if let Ok(vk) = VerifyingKey::from_bytes(&signer.public_key()) {
            self.keys
                .insert((signer.did.clone(), signer.key_id.clone()), vk);
        }
    }
}

impl Verifier for StaticVerifier {
    fn verify(
        &self,
        did: &str,
        key_id: &str,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), VerifyError> {
        let vk = self
            .keys
            .get(&(did.to_owned(), key_id.to_owned()))
            .ok_or_else(|| VerifyError(format!("no key {key_id} for {did}")))?;
        let sig = Signature::from_slice(signature).map_err(|e| VerifyError(e.to_string()))?;
        vk.verify(data, &sig)
            .map_err(|e| VerifyError(e.to_string()))
    }
}
