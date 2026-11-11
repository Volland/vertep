//! Key management: the [`KeyManager`] seam that holds the private keys of DID actors, and its
//! in-memory and file-backed implementations.
//!
//! Private keys never leave a key manager. Callers name a key by its [key name](key_name) (the
//! multikey of its public key, which is not secret) and ask the manager to sign, to agree on a
//! shared secret with a peer, or to open an HPKE ciphertext addressed to the key. Journals,
//! snapshots, datamodels and the graph hold only public keys and key ids (design D3). A hardware
//! module or a remote key service plugs in by implementing the same trait.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use hpke::{Deserializable as _, Serializable as _};
use x25519_dalek::StaticSecret;
use zeroize::Zeroizing;

/// Multicodec prefix of an Ed25519 public key (`ed25519-pub`, 0xed).
pub const ED25519_PUB_CODEC: [u8; 2] = [0xed, 0x01];
/// Multicodec prefix of an X25519 public key (`x25519-pub`, 0xec).
pub const X25519_PUB_CODEC: [u8; 2] = [0xec, 0x01];

/// The key algorithms this crate's built-in key managers hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyType {
    /// Ed25519 signing keys (also usable for X25519 key agreement through the birational map,
    /// as `did:key` requires).
    Ed25519,
    /// X25519 key-agreement keys.
    X25519,
}

impl KeyType {
    /// The name used in key files and the graph.
    pub fn as_str(self) -> &'static str {
        match self {
            KeyType::Ed25519 => "Ed25519",
            KeyType::X25519 => "X25519",
        }
    }

    /// Parses [`KeyType::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "Ed25519" => Some(KeyType::Ed25519),
            "X25519" => Some(KeyType::X25519),
            _ => None,
        }
    }
}

/// A key-management failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// No key has this name.
    UnknownKey(String),
    /// The key cannot perform the operation (for example signing with an X25519 key).
    WrongKeyType(String),
    /// Malformed key material or encoding.
    InvalidKey(String),
    /// A cryptographic operation failed (bad signature input, failed decryption, low-order
    /// point).
    Crypto(String),
    /// The backing store failed.
    Storage(String),
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::UnknownKey(n) => write!(f, "no key named {n}"),
            KeyError::WrongKeyType(n) => write!(f, "key {n} does not perform this operation"),
            KeyError::InvalidKey(m) => write!(f, "invalid key: {m}"),
            KeyError::Crypto(m) => write!(f, "cryptographic failure: {m}"),
            KeyError::Storage(m) => write!(f, "key storage failure: {m}"),
        }
    }
}

impl std::error::Error for KeyError {}

/// A public key: its algorithm and its 32 raw bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey {
    /// The algorithm.
    pub key_type: KeyType,
    /// The raw public key.
    pub bytes: [u8; 32],
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PublicKey({}, {})",
            self.key_type.as_str(),
            self.multikey()
        )
    }
}

impl PublicKey {
    /// An Ed25519 public key.
    pub fn ed25519(bytes: [u8; 32]) -> Self {
        PublicKey {
            key_type: KeyType::Ed25519,
            bytes,
        }
    }

    /// An X25519 public key.
    pub fn x25519(bytes: [u8; 32]) -> Self {
        PublicKey {
            key_type: KeyType::X25519,
            bytes,
        }
    }

    /// The multibase (base58btc) multikey: `z` + base58(multicodec prefix ‖ key).
    pub fn multikey(&self) -> String {
        let prefix = match self.key_type {
            KeyType::Ed25519 => ED25519_PUB_CODEC,
            KeyType::X25519 => X25519_PUB_CODEC,
        };
        let mut b = Vec::with_capacity(34);
        b.extend_from_slice(&prefix);
        b.extend_from_slice(&self.bytes);
        format!("z{}", bs58::encode(b).into_string())
    }

    /// Parses a base58btc multikey of an Ed25519 or X25519 public key.
    ///
    /// # Errors
    ///
    /// [`KeyError::InvalidKey`] for another multibase, another multicodec or a wrong length.
    pub fn from_multikey(s: &str) -> Result<Self, KeyError> {
        let body = s
            .strip_prefix('z')
            .ok_or_else(|| KeyError::InvalidKey(format!("{s}: not base58btc multibase")))?;
        let raw = bs58::decode(body)
            .into_vec()
            .map_err(|e| KeyError::InvalidKey(format!("{s}: {e}")))?;
        if raw.len() != 34 {
            return Err(KeyError::InvalidKey(format!("{s}: wrong length")));
        }
        let key_type = match [raw[0], raw[1]] {
            ED25519_PUB_CODEC => KeyType::Ed25519,
            X25519_PUB_CODEC => KeyType::X25519,
            _ => return Err(KeyError::InvalidKey(format!("{s}: unsupported multicodec"))),
        };
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&raw[2..]);
        Ok(PublicKey { key_type, bytes })
    }

    /// The key as a JWK (`OKP`).
    pub fn to_jwk(&self) -> serde_json::Value {
        serde_json::json!({
            "kty": "OKP",
            "crv": self.key_type.as_str(),
            "x": URL_SAFE_NO_PAD.encode(self.bytes),
        })
    }

    /// Parses an `OKP` JWK with curve `Ed25519` or `X25519`. Private members are refused.
    ///
    /// # Errors
    ///
    /// [`KeyError::InvalidKey`] for other key types, a private `d` member or bad encoding.
    pub fn from_jwk(jwk: &serde_json::Value) -> Result<Self, KeyError> {
        if jwk.get("d").is_some() {
            return Err(KeyError::InvalidKey("JWK carries a private key".into()));
        }
        if jwk.get("kty").and_then(|v| v.as_str()) != Some("OKP") {
            return Err(KeyError::InvalidKey("JWK is not OKP".into()));
        }
        let key_type = jwk
            .get("crv")
            .and_then(|v| v.as_str())
            .and_then(KeyType::parse)
            .ok_or_else(|| KeyError::InvalidKey("unsupported JWK curve".into()))?;
        let x = jwk
            .get("x")
            .and_then(|v| v.as_str())
            .ok_or_else(|| KeyError::InvalidKey("JWK has no x".into()))?;
        let raw = URL_SAFE_NO_PAD
            .decode(x)
            .map_err(|e| KeyError::InvalidKey(format!("JWK x: {e}")))?;
        let bytes: [u8; 32] = raw
            .try_into()
            .map_err(|_| KeyError::InvalidKey("JWK x is not 32 bytes".into()))?;
        Ok(PublicKey { key_type, bytes })
    }

    /// The X25519 form of this key: itself for X25519, the Montgomery form for Ed25519.
    ///
    /// # Errors
    ///
    /// [`KeyError::InvalidKey`] when the Ed25519 bytes are not a valid point.
    pub fn to_x25519(&self) -> Result<PublicKey, KeyError> {
        match self.key_type {
            KeyType::X25519 => Ok(*self),
            KeyType::Ed25519 => {
                let vk = VerifyingKey::from_bytes(&self.bytes)
                    .map_err(|e| KeyError::InvalidKey(e.to_string()))?;
                Ok(PublicKey::x25519(vk.to_montgomery().to_bytes()))
            }
        }
    }

    /// Verifies an Ed25519 signature strictly (no malleable or small-order keys).
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> bool {
        if self.key_type != KeyType::Ed25519 {
            return false;
        }
        let Ok(vk) = VerifyingKey::from_bytes(&self.bytes) else {
            return false;
        };
        let Ok(sig) = ed25519_dalek::Signature::from_slice(signature) else {
            return false;
        };
        vk.verify_strict(message, &sig).is_ok()
    }
}

/// The name of a key in every key manager: the multikey of its public key. It identifies the key
/// without revealing it.
pub fn key_name(public: &PublicKey) -> String {
    public.multikey()
}

/// A raw X25519 shared secret, zeroized on drop.
pub struct SharedSecret(Zeroizing<[u8; 32]>);

impl SharedSecret {
    /// Wraps raw bytes.
    pub fn new(bytes: [u8; 32]) -> Self {
        SharedSecret(Zeroizing::new(bytes))
    }

    /// The secret bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for SharedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedSecret([REDACTED])")
    }
}

/// An RFC 9180 HPKE base-mode ciphertext (DHKEM(X25519, HKDF-SHA256), HKDF-SHA256,
/// ChaCha20-Poly1305) addressed to one public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedBox {
    /// The encapsulated ephemeral public key.
    pub encapsulated: [u8; 32],
    /// The AEAD ciphertext with its tag.
    pub ciphertext: Vec<u8>,
}

type HpkeKem = hpke::kem::X25519HkdfSha256;
type HpkeKdf = hpke::kdf::HkdfSha256;
type HpkeAead = hpke::aead::ChaCha20Poly1305;

/// Seals `plaintext` to `recipient` with HPKE base mode. Only the holder of the recipient's
/// private key can open it, through [`KeyManager::decrypt`].
///
/// # Errors
///
/// [`KeyError`] when the recipient key is not a valid X25519 (or convertible Ed25519) key.
pub fn seal_to(
    recipient: &PublicKey,
    info: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<SealedBox, KeyError> {
    let x = recipient.to_x25519()?;
    let pk = <HpkeKem as hpke::Kem>::PublicKey::from_bytes(&x.bytes)
        .map_err(|e| KeyError::InvalidKey(e.to_string()))?;
    let (enc, ciphertext) = hpke::single_shot_seal::<HpkeAead, HpkeKdf, HpkeKem>(
        &hpke::OpModeS::Base,
        &pk,
        info,
        plaintext,
        aad,
    )
    .map_err(|e| KeyError::Crypto(e.to_string()))?;
    let enc_bytes = enc.to_bytes();
    let mut encapsulated = [0u8; 32];
    encapsulated.copy_from_slice(enc_bytes.as_slice());
    Ok(SealedBox {
        encapsulated,
        ciphertext,
    })
}

/// The key-management seam: every private-key operation of a DID actor goes through it.
///
/// Implementations must never return private key material. The built-in ones are
/// [`InMemoryKeyManager`] and [`FileKeyManager`].
// @tg: implements:: [[openspec:did-identity#Key Management Interface]]
pub trait KeyManager: Send + Sync {
    /// Creates a new key of `key_type` and returns its [name](key_name).
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the key cannot be created or stored.
    fn generate(&self, key_type: KeyType) -> Result<String, KeyError>;

    /// The public key of the key `name`.
    ///
    /// # Errors
    ///
    /// [`KeyError::UnknownKey`] when there is no such key.
    fn public_key(&self, name: &str) -> Result<PublicKey, KeyError>;

    /// An Ed25519 signature over `data` by the key `name`.
    ///
    /// # Errors
    ///
    /// [`KeyError`] for unknown keys and keys that do not sign.
    fn sign(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, KeyError>;

    /// The raw X25519 shared secret between the key `name` and `peer`. Ed25519 keys on either
    /// side are converted to X25519 first, as `did:key` documents do.
    ///
    /// # Errors
    ///
    /// [`KeyError`] for unknown keys, invalid peers and non-contributory results.
    fn key_agreement(&self, name: &str, peer: &PublicKey) -> Result<SharedSecret, KeyError>;

    /// Opens an HPKE base-mode ciphertext addressed to the key `name` (see [`seal_to`]).
    ///
    /// # Errors
    ///
    /// [`KeyError::Crypto`] when the ciphertext does not open with this key.
    fn decrypt(
        &self,
        name: &str,
        sealed: &SealedBox,
        info: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, KeyError>;

    /// Destroys the key `name`.
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the store cannot delete it.
    fn delete(&self, name: &str) -> Result<(), KeyError>;

    /// True when the key `name` exists.
    fn contains(&self, name: &str) -> bool {
        self.public_key(name).is_ok()
    }
}

/// A private key held by a software key manager. Zeroized on drop.
enum Secret {
    Ed25519(SigningKey),
    X25519(StaticSecret),
}

impl Secret {
    fn generate(key_type: KeyType) -> Result<Self, KeyError> {
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(seed.as_mut()).map_err(|e| KeyError::Crypto(e.to_string()))?;
        Ok(Secret::from_seed(key_type, &seed))
    }

    fn from_seed(key_type: KeyType, seed: &[u8; 32]) -> Self {
        match key_type {
            KeyType::Ed25519 => Secret::Ed25519(SigningKey::from_bytes(seed)),
            KeyType::X25519 => Secret::X25519(StaticSecret::from(*seed)),
        }
    }

    fn key_type(&self) -> KeyType {
        match self {
            Secret::Ed25519(_) => KeyType::Ed25519,
            Secret::X25519(_) => KeyType::X25519,
        }
    }

    fn seed(&self) -> Zeroizing<[u8; 32]> {
        match self {
            Secret::Ed25519(k) => Zeroizing::new(k.to_bytes()),
            Secret::X25519(k) => Zeroizing::new(k.to_bytes()),
        }
    }

    fn public(&self) -> PublicKey {
        match self {
            Secret::Ed25519(k) => PublicKey::ed25519(k.verifying_key().to_bytes()),
            Secret::X25519(k) => PublicKey::x25519(x25519_dalek::PublicKey::from(k).to_bytes()),
        }
    }

    /// The X25519 secret of this key (converted for Ed25519 keys).
    fn agreement_secret(&self) -> StaticSecret {
        match self {
            Secret::Ed25519(k) => StaticSecret::from(k.to_scalar_bytes()),
            Secret::X25519(k) => k.clone(),
        }
    }

    fn sign(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, KeyError> {
        match self {
            Secret::Ed25519(k) => Ok(k.sign(data).to_bytes().to_vec()),
            Secret::X25519(_) => Err(KeyError::WrongKeyType(name.to_owned())),
        }
    }

    fn key_agreement(&self, peer: &PublicKey) -> Result<SharedSecret, KeyError> {
        let peer = peer.to_x25519()?;
        let shared = self
            .agreement_secret()
            .diffie_hellman(&x25519_dalek::PublicKey::from(peer.bytes));
        if !shared.was_contributory() {
            return Err(KeyError::Crypto("non-contributory key agreement".into()));
        }
        Ok(SharedSecret::new(shared.to_bytes()))
    }

    fn decrypt(&self, sealed: &SealedBox, info: &[u8], aad: &[u8]) -> Result<Vec<u8>, KeyError> {
        let sk_bytes = Zeroizing::new(self.agreement_secret().to_bytes());
        let sk = <HpkeKem as hpke::Kem>::PrivateKey::from_bytes(sk_bytes.as_slice())
            .map_err(|e| KeyError::InvalidKey(e.to_string()))?;
        let enc = <HpkeKem as hpke::Kem>::EncappedKey::from_bytes(&sealed.encapsulated)
            .map_err(|e| KeyError::Crypto(e.to_string()))?;
        hpke::single_shot_open::<HpkeAead, HpkeKdf, HpkeKem>(
            &hpke::OpModeR::Base,
            &sk,
            &enc,
            info,
            &sealed.ciphertext,
            aad,
        )
        .map_err(|e| KeyError::Crypto(e.to_string()))
    }
}

/// A key manager that keeps keys in process memory only. Keys are zeroized when deleted or when
/// the manager is dropped.
#[derive(Default)]
pub struct InMemoryKeyManager {
    keys: RwLock<BTreeMap<String, Secret>>,
}

impl fmt::Debug for InMemoryKeyManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self
            .keys
            .read()
            .map(|k| k.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("InMemoryKeyManager")
            .field("keys", &names)
            .finish()
    }
}

impl InMemoryKeyManager {
    /// An empty key manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Imports a key from its 32-byte seed (the Ed25519 secret key or the X25519 scalar) and
    /// returns its name. Meant for deterministic tests and for migrating keys in.
    ///
    /// # Errors
    ///
    /// [`KeyError::Storage`] when the manager's lock is poisoned.
    pub fn import_seed(&self, key_type: KeyType, seed: [u8; 32]) -> Result<String, KeyError> {
        let seed = Zeroizing::new(seed);
        self.insert(Secret::from_seed(key_type, &seed))
    }

    fn insert(&self, secret: Secret) -> Result<String, KeyError> {
        let name = key_name(&secret.public());
        self.keys
            .write()
            .map_err(|_| KeyError::Storage("lock poisoned".into()))?
            .insert(name.clone(), secret);
        Ok(name)
    }

    fn with<R>(
        &self,
        name: &str,
        f: impl FnOnce(&Secret) -> Result<R, KeyError>,
    ) -> Result<R, KeyError> {
        let keys = self
            .keys
            .read()
            .map_err(|_| KeyError::Storage("lock poisoned".into()))?;
        let secret = keys
            .get(name)
            .ok_or_else(|| KeyError::UnknownKey(name.to_owned()))?;
        f(secret)
    }
}

impl KeyManager for InMemoryKeyManager {
    fn generate(&self, key_type: KeyType) -> Result<String, KeyError> {
        self.insert(Secret::generate(key_type)?)
    }

    fn public_key(&self, name: &str) -> Result<PublicKey, KeyError> {
        self.with(name, |s| Ok(s.public()))
    }

    fn sign(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.with(name, |s| s.sign(name, data))
    }

    fn key_agreement(&self, name: &str, peer: &PublicKey) -> Result<SharedSecret, KeyError> {
        self.with(name, |s| s.key_agreement(peer))
    }

    fn decrypt(
        &self,
        name: &str,
        sealed: &SealedBox,
        info: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, KeyError> {
        self.with(name, |s| s.decrypt(sealed, info, aad))
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        self.keys
            .write()
            .map_err(|_| KeyError::Storage("lock poisoned".into()))?
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| KeyError::UnknownKey(name.to_owned()))
    }
}

/// A key manager that keeps one file per key in a directory, readable only by the owner on Unix
/// (mode 0600). Keys are read for each operation and zeroized afterwards; nothing is cached.
///
/// The files are not encrypted at rest: a deployment that needs that uses an OS keystore, a
/// hardware module or a remote key service behind [`KeyManager`].
#[derive(Debug)]
pub struct FileKeyManager {
    dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct KeyFile {
    #[serde(rename = "type")]
    key_type: String,
    secret: String,
}

impl FileKeyManager {
    /// A key manager over `dir`, created if missing.
    ///
    /// # Errors
    ///
    /// [`KeyError::Storage`] when the directory cannot be created.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, KeyError> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| KeyError::Storage(e.to_string()))?;
        Ok(FileKeyManager { dir })
    }

    /// Imports a key from its 32-byte seed and returns its name.
    ///
    /// # Errors
    ///
    /// [`KeyError::Storage`] when the key file cannot be written.
    pub fn import_seed(&self, key_type: KeyType, seed: [u8; 32]) -> Result<String, KeyError> {
        let seed = Zeroizing::new(seed);
        self.store(&Secret::from_seed(key_type, &seed))
    }

    fn path(&self, name: &str) -> Result<PathBuf, KeyError> {
        // Names are base58 multikeys; refuse anything that could escape the directory.
        if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric()) {
            return Err(KeyError::UnknownKey(name.to_owned()));
        }
        Ok(self.dir.join(format!("{name}.key")))
    }

    fn store(&self, secret: &Secret) -> Result<String, KeyError> {
        let name = key_name(&secret.public());
        let seed = secret.seed();
        let file = Zeroizing::new(
            serde_json::to_vec(&KeyFile {
                key_type: secret.key_type().as_str().to_owned(),
                secret: URL_SAFE_NO_PAD.encode(seed.as_slice()),
            })
            .map_err(|e| KeyError::Storage(e.to_string()))?,
        );
        let path = self.path(&name)?;
        write_private(&path, &file).map_err(|e| KeyError::Storage(e.to_string()))?;
        Ok(name)
    }

    fn load(&self, name: &str) -> Result<Secret, KeyError> {
        let path = self.path(name)?;
        let bytes = Zeroizing::new(std::fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                KeyError::UnknownKey(name.to_owned())
            } else {
                KeyError::Storage(e.to_string())
            }
        })?);
        let file: KeyFile =
            serde_json::from_slice(&bytes).map_err(|e| KeyError::Storage(e.to_string()))?;
        let key_type = KeyType::parse(&file.key_type)
            .ok_or_else(|| KeyError::Storage(format!("unknown key type {}", file.key_type)))?;
        let raw = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(file.secret.as_bytes())
                .map_err(|e| KeyError::Storage(e.to_string()))?,
        );
        let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
            raw.as_slice()
                .try_into()
                .map_err(|_| KeyError::Storage("key file holds a wrong-size key".into()))?,
        );
        let secret = Secret::from_seed(key_type, &seed);
        if key_name(&secret.public()) != name {
            return Err(KeyError::Storage(format!(
                "key file {name} does not match its name"
            )));
        }
        Ok(secret)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

impl KeyManager for FileKeyManager {
    fn generate(&self, key_type: KeyType) -> Result<String, KeyError> {
        self.store(&Secret::generate(key_type)?)
    }

    fn public_key(&self, name: &str) -> Result<PublicKey, KeyError> {
        Ok(self.load(name)?.public())
    }

    fn sign(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.load(name)?.sign(name, data)
    }

    fn key_agreement(&self, name: &str, peer: &PublicKey) -> Result<SharedSecret, KeyError> {
        self.load(name)?.key_agreement(peer)
    }

    fn decrypt(
        &self,
        name: &str,
        sealed: &SealedBox,
        info: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, KeyError> {
        self.load(name)?.decrypt(sealed, info, aad)
    }

    fn delete(&self, name: &str) -> Result<(), KeyError> {
        let path = self.path(name)?;
        std::fs::remove_file(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                KeyError::UnknownKey(name.to_owned())
            } else {
                KeyError::Storage(e.to_string())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multikey_round_trip_and_known_vector() {
        // did:key test vector (W3C CCG did-method-key): Ed25519 key z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK.
        let mk = "z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";
        let pk = PublicKey::from_multikey(mk).expect("parses");
        assert_eq!(pk.key_type, KeyType::Ed25519);
        assert_eq!(pk.multikey(), mk);
        let x = pk.to_x25519().expect("converts");
        assert_eq!(
            x.multikey(),
            "z6LSj72tK8brWgZja8NLRwPigth2T9QRiG1uH9oKZuKjdh9p"
        );
    }

    #[test]
    fn agreement_is_symmetric_across_ed25519_and_x25519() {
        let km = InMemoryKeyManager::new();
        let a = km.generate(KeyType::Ed25519).expect("a");
        let b = km.generate(KeyType::X25519).expect("b");
        let pa = km.public_key(&a).expect("pa");
        let pb = km.public_key(&b).expect("pb");
        let s1 = km.key_agreement(&a, &pb).expect("s1");
        let s2 = km.key_agreement(&b, &pa).expect("s2");
        assert_eq!(s1.as_bytes(), s2.as_bytes());
    }

    #[test]
    fn hpke_seal_and_decrypt() {
        let km = InMemoryKeyManager::new();
        let k = km.generate(KeyType::X25519).expect("k");
        let pk = km.public_key(&k).expect("pk");
        let sealed = seal_to(&pk, b"info", b"aad", b"hello").expect("seal");
        assert_eq!(
            km.decrypt(&k, &sealed, b"info", b"aad").expect("open"),
            b"hello"
        );
        assert!(km.decrypt(&k, &sealed, b"info", b"other").is_err());
    }
}
