//! [`KmSigner`]: the `didwebvh-rs` [`Signer`] over Vertep's [`KeyManager`], so update keys,
//! witness keys and object keys never leave the key manager.

use std::fmt;
use std::sync::Arc;

use affinidi_data_integrity::DataIntegrityError;
use async_trait::async_trait;
use didwebvh_rs::prelude::{KeyType as DiKeyType, Secret, Signer};
use vertep_federation::{KeyError, KeyManager, KeyType, PublicKey};

/// The `did:key` DID of an Ed25519 public key: `did:key:<multikey>`.
pub fn did_key(public: &PublicKey) -> String {
    format!("did:key:{}", public.multikey())
}

/// The pre-rotation hash of a multikey: base58btc of the SHA-256 multihash of the multikey
/// string, as `nextKeyHashes` holds it.
///
/// # Panics
///
/// Never: hashing a string cannot fail.
pub fn key_hash(multikey: &str) -> String {
    Secret::base58_hash_string(multikey).unwrap_or_default()
}

/// Signs `did:webvh` log entries and witness proofs with the Ed25519 key `key` of a key manager.
/// Its verification method is `did:key:<multikey>#<multikey>`, as `did:webvh` 1.0 requires.
#[derive(Clone)]
pub struct KmSigner {
    keys: Arc<dyn KeyManager>,
    key: String,
    public: PublicKey,
    vm: String,
}

impl fmt::Debug for KmSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KmSigner")
            .field("key", &self.key)
            .field("vm", &self.vm)
            .finish_non_exhaustive()
    }
}

impl KmSigner {
    /// A signer for the key `key` of `keys`.
    ///
    /// # Errors
    ///
    /// [`KeyError::UnknownKey`] when the key manager has no such key, and
    /// [`KeyError::WrongKeyType`] when it is not an Ed25519 key.
    // @tg: implements:: [[openspec:webvh-identity#Create With Vertep Keys]] {note: "every signature goes through the key manager"}
    pub fn new(keys: Arc<dyn KeyManager>, key: impl Into<String>) -> Result<Self, KeyError> {
        let key = key.into();
        let public = keys.public_key(&key)?;
        if public.key_type != KeyType::Ed25519 {
            return Err(KeyError::WrongKeyType(format!(
                "{key} is not an Ed25519 key"
            )));
        }
        let mk = public.multikey();
        Ok(KmSigner {
            keys,
            key,
            public,
            vm: format!("did:key:{mk}#{mk}"),
        })
    }

    /// The key's name in the key manager.
    pub fn key_name(&self) -> &str {
        &self.key
    }

    /// The public key.
    pub fn public_key(&self) -> PublicKey {
        self.public
    }

    /// The public key as a multikey (what `updateKeys` lists).
    pub fn multikey(&self) -> String {
        self.public.multikey()
    }

    /// The `did:key` DID of the key (what a witness list names).
    pub fn did_key(&self) -> String {
        did_key(&self.public)
    }
}

#[derive(Debug)]
struct SignError(String);

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SignError {}

#[async_trait]
impl Signer for KmSigner {
    fn key_type(&self) -> DiKeyType {
        DiKeyType::Ed25519
    }

    fn verification_method(&self) -> &str {
        &self.vm
    }

    async fn sign(&self, data: &[u8]) -> Result<Vec<u8>, DataIntegrityError> {
        self.keys
            .sign(&self.key, data)
            .map_err(|e| DataIntegrityError::signing(SignError(e.to_string())))
    }
}
