//! The object's own TSP keys, shared by its hosts: generated once by the controller, wrapped
//! to every host's key-agreement key, and installed into each host's key manager, so any host
//! can open messages addressed to the object while only the writer journals and acknowledges
//! them.

use core::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use vertep_federation::keys::{SealedBox, seal_to};
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType, PublicKey};

use super::wire::WireError;

const SHARE_INFO: &[u8] = b"vertep-object-key-v1";

/// The seeds of an object's signing (Ed25519) and key-agreement (X25519) keys. Only the
/// controller holds them, while it wraps them to hosts.
pub struct ObjectKeys {
    signing: [u8; 32],
    agreement: [u8; 32],
}

impl fmt::Debug for ObjectKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectKeys").finish_non_exhaustive()
    }
}

impl Drop for ObjectKeys {
    fn drop(&mut self) {
        self.signing = [0; 32];
        self.agreement = [0; 32];
    }
}

/// An object's keys wrapped to one host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectKeyShare {
    /// The object.
    pub object: String,
    /// The host it is wrapped to.
    pub host: String,
    /// The wrapped signing seed (encapsulated key, ciphertext).
    pub signing: (String, String),
    /// The wrapped agreement seed.
    pub agreement: (String, String),
}

/// The names of an object's keys after installation in a host's key manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledObjectKeys {
    /// The signing key's name.
    pub signing: String,
    /// The agreement key's name.
    pub agreement: String,
}

fn wrap(seed: &[u8; 32], object: &str, to: &PublicKey) -> Result<(String, String), WireError> {
    let s =
        seal_to(to, SHARE_INFO, object.as_bytes(), seed).map_err(|e| WireError(e.to_string()))?;
    Ok((
        URL_SAFE_NO_PAD.encode(s.encapsulated),
        URL_SAFE_NO_PAD.encode(s.ciphertext),
    ))
}

fn unwrap(
    w: &(String, String),
    object: &str,
    keys: &dyn KeyManager,
    key: &str,
) -> Result<[u8; 32], WireError> {
    let dec = |s: &str| {
        URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|e| WireError(e.to_string()))
    };
    let encapsulated: [u8; 32] = dec(&w.0)?
        .try_into()
        .map_err(|_| WireError("encapsulated key is not 32 bytes".into()))?;
    let sealed = SealedBox {
        encapsulated,
        ciphertext: dec(&w.1)?,
    };
    keys.decrypt(key, &sealed, SHARE_INFO, object.as_bytes())
        .map_err(|e| WireError(format!("object key share: {e}")))?
        .try_into()
        .map_err(|_| WireError("object key seed is not 32 bytes".into()))
}

impl ObjectKeys {
    /// Fresh keys.
    ///
    /// # Errors
    ///
    /// [`WireError`] when the random source fails.
    pub fn generate() -> Result<Self, WireError> {
        let mut k = ObjectKeys {
            signing: [0; 32],
            agreement: [0; 32],
        };
        getrandom::fill(&mut k.signing).map_err(|e| WireError(e.to_string()))?;
        getrandom::fill(&mut k.agreement).map_err(|e| WireError(e.to_string()))?;
        Ok(k)
    }

    /// The public keys (signing, agreement), for the object's DID document.
    ///
    /// # Errors
    ///
    /// [`WireError`] when the seeds do not make keys.
    pub fn public_keys(&self) -> Result<(PublicKey, PublicKey), WireError> {
        let km = InMemoryKeyManager::new();
        let e = |e: vertep_federation::KeyError| WireError(e.to_string());
        let s = km.import_seed(KeyType::Ed25519, self.signing).map_err(e)?;
        let a = km.import_seed(KeyType::X25519, self.agreement).map_err(e)?;
        Ok((km.public_key(&s).map_err(e)?, km.public_key(&a).map_err(e)?))
    }

    /// The keys wrapped to `host`'s key-agreement key `to`.
    ///
    /// # Errors
    ///
    /// [`WireError`] when `to` is not a key-agreement key.
    // @tg: implements:: [[decisions#Hosts share the object VID and only the writer acknowledges]]
    pub fn share_with(
        &self,
        object: &str,
        host: &str,
        to: &PublicKey,
    ) -> Result<ObjectKeyShare, WireError> {
        Ok(ObjectKeyShare {
            object: object.to_owned(),
            host: host.to_owned(),
            signing: wrap(&self.signing, object, to)?,
            agreement: wrap(&self.agreement, object, to)?,
        })
    }
}

impl ObjectKeyShare {
    /// Opens the share with the host's key-agreement key `key` of `keys` and installs the
    /// object's keys into `into`.
    ///
    /// # Errors
    ///
    /// [`WireError`] when the share does not open or the keys cannot be stored.
    pub fn install(
        &self,
        keys: &dyn KeyManager,
        key: &str,
        into: &InMemoryKeyManager,
    ) -> Result<InstalledObjectKeys, WireError> {
        let mut s = unwrap(&self.signing, &self.object, keys, key)?;
        let mut a = unwrap(&self.agreement, &self.object, keys, key)?;
        let e = |e: vertep_federation::KeyError| WireError(e.to_string());
        let out = InstalledObjectKeys {
            signing: into.import_seed(KeyType::Ed25519, s).map_err(e)?,
            agreement: into.import_seed(KeyType::X25519, a).map_err(e)?,
        };
        s = [0; 32];
        a = [0; 32];
        let _ = (s, a);
        Ok(out)
    }
}
