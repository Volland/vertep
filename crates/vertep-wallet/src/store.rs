//! The sealed credential store (`agent-wallet` requirement `Sealed Credential Store`, design
//! D8).
//!
//! Every record (a credential with its metadata) is encrypted with XChaCha20-Poly1305 under
//! the wallet's content key, with the credential digest as associated data, so a record moved
//! or altered does not decrypt. The content key is sealed with HPKE to the wallet's
//! key-agreement key and opened only by the key manager. Records live in a [`RecordBackend`]:
//! memory, or a directory.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use vertep_federation::KeyManager;
use vertep_federation::keys::{SealedBox, seal_to};
use zeroize::Zeroizing;

use crate::credential::{Credential, CredentialMeta};
use crate::error::WalletError;
use crate::plugin::CredentialStore;

const KEY_RECORD: &str = "content-key";
const HPKE_INFO: &[u8] = b"vertep-wallet content key";

/// Where sealed records are kept.
pub trait RecordBackend: Send + Sync {
    /// Writes `bytes` under `name`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), WalletError>;
    /// The bytes under `name`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, WalletError>;
    /// Deletes `name`.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    fn delete(&self, name: &str) -> Result<(), WalletError>;
    /// Every name.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`].
    fn names(&self) -> Result<Vec<String>, WalletError>;
}

/// Records in memory.
#[derive(Clone, Debug, Default)]
pub struct MemoryRecords(Arc<Mutex<BTreeMap<String, Vec<u8>>>>);

impl MemoryRecords {
    /// An empty backend.
    pub fn new() -> Self {
        Self::default()
    }
}

impl RecordBackend for MemoryRecords {
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), WalletError> {
        self.0
            .lock()
            .map_err(|_| WalletError::Storage("poisoned".into()))?
            .insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, WalletError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| WalletError::Storage("poisoned".into()))?
            .get(name)
            .cloned())
    }
    fn delete(&self, name: &str) -> Result<(), WalletError> {
        self.0
            .lock()
            .map_err(|_| WalletError::Storage("poisoned".into()))?
            .remove(name);
        Ok(())
    }
    fn names(&self) -> Result<Vec<String>, WalletError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| WalletError::Storage("poisoned".into()))?
            .keys()
            .cloned()
            .collect())
    }
}

/// Records as files in a directory.
#[derive(Clone, Debug)]
pub struct DirRecords(PathBuf);

impl DirRecords {
    /// Records in `dir`, created if missing.
    ///
    /// # Errors
    ///
    /// [`WalletError::Storage`] when the directory cannot be created.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, WalletError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| WalletError::Storage(e.to_string()))?;
        Ok(DirRecords(dir))
    }

    fn path(&self, name: &str) -> PathBuf {
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.0.join(safe)
    }
}

impl RecordBackend for DirRecords {
    fn write(&self, name: &str, bytes: &[u8]) -> Result<(), WalletError> {
        let p = self.path(name);
        let tmp = p.with_extension("tmp");
        std::fs::write(&tmp, bytes).map_err(|e| WalletError::Storage(e.to_string()))?;
        std::fs::rename(&tmp, &p).map_err(|e| WalletError::Storage(e.to_string()))
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, WalletError> {
        match std::fs::read(self.path(name)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(WalletError::Storage(e.to_string())),
        }
    }
    fn delete(&self, name: &str) -> Result<(), WalletError> {
        match std::fs::remove_file(self.path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WalletError::Storage(e.to_string())),
        }
    }
    fn names(&self) -> Result<Vec<String>, WalletError> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.0).map_err(|e| WalletError::Storage(e.to_string()))? {
            let e = e.map_err(|e| WalletError::Storage(e.to_string()))?;
            if let Some(n) = e.file_name().to_str()
                && !n.ends_with(".tmp")
            {
                out.push(n.to_owned());
            }
        }
        Ok(out)
    }
}

#[derive(Serialize, Deserialize)]
struct Record {
    meta: CredentialMeta,
    credential: Credential,
}

fn record_name(digest: &str) -> String {
    format!("cred-{}", digest.trim_start_matches("sha256:"))
}

/// The sealed store: records encrypted under a content key only the key manager can unwrap.
// @tg: implements:: [[openspec:agent-wallet#Sealed Credential Store]]
pub struct SealedStore {
    backend: Arc<dyn RecordBackend>,
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SealedStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealedStore")
    }
}

impl SealedStore {
    /// Opens the store in `backend`, unwrapping its content key with the key-agreement key
    /// `agreement_key` (or creating and sealing a new content key for an empty backend).
    ///
    /// # Errors
    ///
    /// [`WalletError::Integrity`] when the content key does not open with this key.
    pub fn open(
        km: &dyn KeyManager,
        agreement_key: &str,
        backend: Arc<dyn RecordBackend>,
    ) -> Result<Self, WalletError> {
        let key = match backend.read(KEY_RECORD)? {
            Some(b) => {
                if b.len() < 32 {
                    return Err(WalletError::Integrity("content key record".into()));
                }
                let mut enc = [0u8; 32];
                enc.copy_from_slice(&b[..32]);
                let sealed = SealedBox {
                    encapsulated: enc,
                    ciphertext: b[32..].to_vec(),
                };
                let k = km
                    .decrypt(agreement_key, &sealed, HPKE_INFO, b"")
                    .map_err(|e| WalletError::Integrity(format!("content key: {e}")))?;
                let mut out = Zeroizing::new([0u8; 32]);
                if k.len() != 32 {
                    return Err(WalletError::Integrity("content key length".into()));
                }
                out.copy_from_slice(&k);
                out
            }
            None => {
                let mut out = Zeroizing::new([0u8; 32]);
                getrandom::fill(out.as_mut()).map_err(|e| WalletError::Storage(e.to_string()))?;
                let public = km.public_key(agreement_key)?;
                let sealed = seal_to(&public, HPKE_INFO, b"", out.as_ref())?;
                let mut bytes = sealed.encapsulated.to_vec();
                bytes.extend_from_slice(&sealed.ciphertext);
                backend.write(KEY_RECORD, &bytes)?;
                out
            }
        };
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|e| WalletError::Storage(e.to_string()))?;
        Ok(SealedStore { backend, cipher })
    }

    fn open_record(&self, digest: &str, bytes: &[u8]) -> Result<Record, WalletError> {
        if bytes.len() < 24 {
            return Err(WalletError::Integrity(format!(
                "record {digest} is truncated"
            )));
        }
        let plain = self
            .cipher
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: digest.as_bytes(),
                },
            )
            .map_err(|_| {
                WalletError::Integrity(format!("record {digest} does not match its digest"))
            })?;
        let rec: Record = serde_json::from_slice(&plain)
            .map_err(|e| WalletError::Integrity(format!("record {digest}: {e}")))?;
        if rec.meta.digest != digest {
            return Err(WalletError::Integrity(format!(
                "record {digest} holds another credential"
            )));
        }
        Ok(rec)
    }
}

impl CredentialStore for SealedStore {
    fn put(&self, meta: &CredentialMeta, credential: &Credential) -> Result<(), WalletError> {
        let plain = serde_json::to_vec(&Record {
            meta: meta.clone(),
            credential: credential.clone(),
        })
        .map_err(|e| WalletError::Storage(e.to_string()))?;
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|e| WalletError::Storage(e.to_string()))?;
        let ct = self
            .cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: meta.digest.as_bytes(),
                },
            )
            .map_err(|e| WalletError::Storage(e.to_string()))?;
        let mut bytes = nonce.to_vec();
        bytes.extend_from_slice(&ct);
        self.backend.write(&record_name(&meta.digest), &bytes)
    }

    fn get(&self, digest: &str) -> Result<(CredentialMeta, Credential), WalletError> {
        let bytes = self
            .backend
            .read(&record_name(digest))?
            .ok_or_else(|| WalletError::UnknownCredential(digest.to_owned()))?;
        let r = self.open_record(digest, &bytes)?;
        Ok((r.meta, r.credential))
    }

    fn list(&self) -> Result<Vec<CredentialMeta>, WalletError> {
        let mut out = Vec::new();
        for n in self.backend.names()? {
            let Some(hex) = n.strip_prefix("cred-") else {
                continue;
            };
            let digest = format!("sha256:{hex}");
            if let Some(b) = self.backend.read(&n)?
                && let Ok(r) = self.open_record(&digest, &b)
            {
                out.push(r.meta);
            }
        }
        Ok(out)
    }

    fn erase(&self, digest: &str) -> Result<(), WalletError> {
        self.backend.delete(&record_name(digest))
    }
}
