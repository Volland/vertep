//! The custody protocol's messages, the wire form of journal entries, the key directory and
//! the keeper encryption of entries.
//!
//! Messages are JSON. They travel between a host and a custodian over any
//! [`CustodyTransport`](super::link::CustodyTransport): in process, or as TSP application
//! messages between the two parties' VIDs.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use oxrdf::{Dataset, NamedNode};
use oxttl::{NQuadsParser, NQuadsSerializer};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use vertep_federation::keys::{SealedBox, seal_to};
use vertep_federation::{Did, DidResolver, KeyError, KeyManager, PublicKey};
use vertep_store::Entry;

use super::config::CustodyConfig;
use crate::chain::{ChainError, Digest, EntrySigner, content_hash, read_link};

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(s: &str) -> Result<Vec<u8>, WireError> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| WireError(format!("base64: {e}")))
}

/// A malformed message or entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError(pub String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "custody wire: {}", self.0)
    }
}

impl std::error::Error for WireError {}

impl From<ChainError> for WireError {
    fn from(e: ChainError) -> Self {
        WireError(e.to_string())
    }
}

/// An entry as N-Quads text (one named graph).
pub fn entry_to_nquads(entry: &Entry) -> String {
    let mut w = NQuadsSerializer::new().for_writer(Vec::new());
    for q in entry.quads() {
        // Writing to a Vec cannot fail.
        let _ = w.serialize_quad(&q);
    }
    String::from_utf8(w.finish()).unwrap_or_default()
}

/// The entry encoded by N-Quads text.
///
/// # Errors
///
/// [`WireError`] for a syntax error or a dataset that is not one entry.
pub fn entry_from_nquads(text: &str) -> Result<Entry, WireError> {
    let mut ds = Dataset::new();
    for q in NQuadsParser::new().for_slice(text.as_bytes()) {
        ds.insert(&q.map_err(|e| WireError(format!("n-quads: {e}")))?);
    }
    Entry::from_dataset(&ds).map_err(|e| WireError(e.to_string()))
}

/// What a keeper sees of an entry in clear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryHeader {
    /// The object.
    pub object: String,
    /// The entry's position.
    pub position: u64,
    /// The epoch the entry was written in.
    pub epoch: u64,
    /// The writing host.
    pub writer: String,
    /// The previous entry's content hash.
    pub prev: Option<String>,
    /// The entry's content hash.
    pub hash: String,
    /// The content-key generation the body is encrypted under.
    pub generation: u64,
    /// The writer's signature over the content hash (the entry's `vt:entrySignature`), which a
    /// keeper verifies without reading the body.
    pub entry_signature: String,
}

/// An entry as a keeper stores it: a clear header, an encrypted body, and the signature of the
/// host that encrypted it (the writer that shipped it, which for an inherited entry is not the
/// entry's writer) over both.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperEntry {
    /// The clear header.
    pub header: EntryHeader,
    /// The AEAD nonce.
    pub nonce: String,
    /// The encrypted N-Quads of the sealed entry.
    pub ciphertext: String,
    /// The host that encrypted the body.
    pub sealer: String,
    /// The sealer's signature over the header and the ciphertext.
    pub signature: String,
}

/// An entry on the wire: in clear to hosts, encrypted to keepers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "lowercase")]
#[allow(clippy::large_enum_variant)] // the JSON wire form; boxing would not shrink messages
pub enum WireEntry {
    /// The sealed entry as N-Quads.
    Clear {
        /// The N-Quads text.
        nquads: String,
    },
    /// The keeper form.
    Sealed(KeeperEntry),
}

/// The custody facts of a wire entry that every custodian can read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireFacts {
    /// Position.
    pub position: u64,
    /// Written epoch.
    pub epoch: u64,
    /// Writer.
    pub writer: String,
    /// Previous hash.
    pub prev: Option<Digest>,
    /// Content hash.
    pub hash: Digest,
}

impl WireEntry {
    /// The clear form of a sealed entry.
    pub fn clear(entry: &Entry) -> Self {
        WireEntry::Clear {
            nquads: entry_to_nquads(entry),
        }
    }

    /// The custody facts: from the entry's own link (clear) or the header (keeper form). For
    /// the clear form the hash is recomputed from the content.
    ///
    /// # Errors
    ///
    /// [`WireError`] for a malformed entry or header.
    pub fn facts(&self) -> Result<WireFacts, WireError> {
        match self {
            WireEntry::Clear { nquads } => {
                let e = entry_from_nquads(nquads)?;
                let l = read_link(&e)?;
                Ok(WireFacts {
                    position: l.link.position.get(),
                    epoch: l.link.epoch,
                    writer: l.link.writer,
                    prev: l.link.prev,
                    hash: content_hash(&e),
                })
            }
            WireEntry::Sealed(k) => {
                let h = &k.header;
                let parse =
                    |s: &str| Digest::from_hex(s).ok_or_else(|| WireError(format!("bad hash {s}")));
                Ok(WireFacts {
                    position: h.position,
                    epoch: h.epoch,
                    writer: h.writer.clone(),
                    prev: h.prev.as_deref().map(parse).transpose()?,
                    hash: parse(&h.hash)?,
                })
            }
        }
    }
}

/// A content key wrapped to one host's key-agreement key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyWrap {
    /// The key generation.
    pub generation: u64,
    /// The host it is wrapped to.
    pub host: String,
    /// The HPKE encapsulated key.
    pub encapsulated: String,
    /// The HPKE ciphertext of the content key.
    pub ciphertext: String,
}

const KEY_INFO: &[u8] = b"vertep-content-key-v1";

/// A per-object symmetric key for keeper replicas.
#[derive(Clone, PartialEq, Eq)]
pub struct ContentKey {
    /// The generation, from 1; a new generation starts when a host leaves.
    pub generation: u64,
    key: [u8; 32],
}

impl fmt::Debug for ContentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentKey")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl ContentKey {
    /// A fresh random key of `generation`.
    ///
    /// # Errors
    ///
    /// [`WireError`] when the system random source fails.
    pub fn generate(generation: u64) -> Result<Self, WireError> {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| WireError(format!("random: {e}")))?;
        Ok(ContentKey { generation, key })
    }

    fn aad(object: &str, generation: u64) -> Vec<u8> {
        format!("{object}\n{generation}").into_bytes()
    }

    /// The key wrapped to `host`'s key-agreement key.
    ///
    /// # Errors
    ///
    /// [`WireError`] when the key cannot be sealed to `agreement`.
    pub fn wrap_for(
        &self,
        object: &str,
        host: &str,
        agreement: &PublicKey,
    ) -> Result<KeyWrap, WireError> {
        let sealed = seal_to(
            agreement,
            KEY_INFO,
            &Self::aad(object, self.generation),
            &self.key,
        )
        .map_err(|e| WireError(e.to_string()))?;
        Ok(KeyWrap {
            generation: self.generation,
            host: host.to_owned(),
            encapsulated: b64(&sealed.encapsulated),
            ciphertext: b64(&sealed.ciphertext),
        })
    }

    /// Opens `wrap` with the key-agreement key `key` of `keys`.
    ///
    /// # Errors
    ///
    /// [`WireError`] when the wrap does not open.
    pub fn unwrap(
        object: &str,
        wrap: &KeyWrap,
        keys: &dyn KeyManager,
        key: &str,
    ) -> Result<Self, WireError> {
        let enc: [u8; 32] = unb64(&wrap.encapsulated)?
            .try_into()
            .map_err(|_| WireError("encapsulated key is not 32 bytes".into()))?;
        let sealed = SealedBox {
            encapsulated: enc,
            ciphertext: unb64(&wrap.ciphertext)?,
        };
        let plain = keys
            .decrypt(key, &sealed, KEY_INFO, &Self::aad(object, wrap.generation))
            .map_err(|e| WireError(format!("content key: {e}")))?;
        let key: [u8; 32] = plain
            .try_into()
            .map_err(|_| WireError("content key is not 32 bytes".into()))?;
        Ok(ContentKey {
            generation: wrap.generation,
            key,
        })
    }
}

fn header_signing_input(header: &EntryHeader, ciphertext: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(header).unwrap_or_default());
    h.update(b"\n");
    h.update(ciphertext);
    let mut v = b"vertep-keeper-entry-v1\n".to_vec();
    v.extend_from_slice(&h.finalize());
    v
}

/// The keeper form of a sealed entry: its header in clear, its N-Quads encrypted under `key`
/// and the header and ciphertext signed by `signer` (the entry's writer).
///
/// # Errors
///
/// [`WireError`] for an unsealed entry, a random-source failure or a signing failure.
// @tg: implements:: [[openspec:object-custody#Encrypted Replicas At Keepers]]
pub fn seal_for_keeper(
    entry: &Entry,
    key: &ContentKey,
    signer: &dyn EntrySigner,
) -> Result<KeeperEntry, WireError> {
    let l = read_link(entry)?;
    let header = EntryHeader {
        object: l.link.object.as_str().to_owned(),
        position: l.link.position.get(),
        epoch: l.link.epoch,
        writer: l.link.writer.clone(),
        prev: l.link.prev.map(|d| d.to_hex()),
        hash: l.hash.to_hex(),
        generation: key.generation,
        entry_signature: b64(&l.signature),
    };
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|e| WireError(format!("random: {e}")))?;
    let aad = serde_json::to_vec(&header).unwrap_or_default();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.key));
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: entry_to_nquads(entry).as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|e| WireError(format!("encrypt: {e}")))?;
    let signature = signer
        .sign(&header_signing_input(&header, &ciphertext))
        .map_err(|e| WireError(e.to_string()))?;
    Ok(KeeperEntry {
        header,
        nonce: b64(&nonce),
        ciphertext: b64(&ciphertext),
        sealer: signer.writer().to_owned(),
        signature: b64(&signature),
    })
}

/// True when the entry's writer signed its content hash: `writer_keys` are the keys of
/// `entry.header.writer`.
pub fn verify_keeper_authorship(entry: &KeeperEntry, writer_keys: &[PublicKey]) -> bool {
    let (Ok(sig), Some(hash)) = (
        unb64(&entry.header.entry_signature),
        Digest::from_hex(&entry.header.hash),
    ) else {
        return false;
    };
    let input = crate::chain::signing_input(&hash);
    writer_keys.iter().any(|k| k.verify(&input, &sig))
}

/// True when the sealer signed the header and ciphertext: `sealer_keys` are the keys of
/// `entry.sealer`.
pub fn verify_keeper_seal(entry: &KeeperEntry, sealer_keys: &[PublicKey]) -> bool {
    let (Ok(ct), Ok(sig)) = (unb64(&entry.ciphertext), unb64(&entry.signature)) else {
        return false;
    };
    let input = header_signing_input(&entry.header, &ct);
    sealer_keys.iter().any(|k| k.verify(&input, &sig))
}

/// Decrypts a keeper entry with `key` and checks that the content matches the header's hash.
///
/// # Errors
///
/// [`WireError`] when it does not decrypt, does not parse or does not match its hash.
pub fn open_keeper_entry(entry: &KeeperEntry, key: &ContentKey) -> Result<Entry, WireError> {
    if entry.header.generation != key.generation {
        return Err(WireError(format!(
            "entry {} is under key generation {}, not {}",
            entry.header.position, entry.header.generation, key.generation
        )));
    }
    let nonce = unb64(&entry.nonce)?;
    if nonce.len() != 12 {
        return Err(WireError("nonce is not 12 bytes".into()));
    }
    let aad = serde_json::to_vec(&entry.header).unwrap_or_default();
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.key));
    let plain = cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &unb64(&entry.ciphertext)?,
                aad: &aad,
            },
        )
        .map_err(|_| WireError(format!("entry {} does not decrypt", entry.header.position)))?;
    let text = String::from_utf8(plain).map_err(|e| WireError(e.to_string()))?;
    let e = entry_from_nquads(&text)?;
    if content_hash(&e).to_hex() != entry.header.hash {
        return Err(WireError(format!(
            "entry {} does not match its header hash",
            entry.header.position
        )));
    }
    Ok(e)
}

/// A custodian's signed promise not to accept anything from an epoch lower than `epoch`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochPromise {
    /// The object.
    pub object: String,
    /// The promised epoch.
    pub epoch: u64,
    /// The promising custodian.
    pub custodian: String,
    /// The custodian's journal head.
    pub head: u64,
    /// The hash of the entry at `head`.
    pub head_hash: Option<String>,
    /// The epoch in which the custodian accepted its journal (0 when it never did).
    pub accepted_epoch: u64,
    /// The custody configuration the custodian accepted.
    pub config: CustodyConfig,
    /// The pending configuration of a change in progress (joint consensus), if any.
    #[serde(default)]
    pub next: Option<CustodyConfig>,
    /// The custodian's signature over the other fields.
    pub signature: String,
}

/// The rank of a configuration record: a single configuration `v` is below the joint record
/// `v` with `v+1` pending, which is below the single configuration `v+1`.
pub fn record_rank(config: &CustodyConfig, next: Option<&CustodyConfig>) -> u64 {
    config.version * 2 + u64::from(next.is_some())
}

/// A batch of entries a writer ships to a custodian.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendBatch {
    /// The object.
    pub object: String,
    /// The writer's epoch.
    pub epoch: u64,
    /// The writer.
    pub writer: String,
    /// The position the first entry follows.
    pub after: u64,
    /// The entries, in order, up to the writer's head.
    pub entries: Vec<WireEntry>,
    /// The effective custody configuration the writer holds.
    pub config: CustodyConfig,
    /// The configuration the writer is changing to, while a change is in progress.
    #[serde(default)]
    pub next: Option<CustodyConfig>,
    /// Content keys wrapped to hosts.
    pub keys: Vec<KeyWrap>,
    /// The writer's signature over the other fields.
    pub signature: String,
}

/// A request to a custodian.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CustodyRequest {
    /// `https://w3id.org/vertep/custody/1.0#prepare`: promise `epoch` to `candidate`.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#prepare")]
    Prepare {
        /// The object.
        object: String,
        /// The epoch asked for.
        epoch: u64,
        /// The candidate host.
        candidate: String,
    },
    /// `https://w3id.org/vertep/custody/1.0#append`: store a batch.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#append")]
    Append(AppendBatch),
    /// `https://w3id.org/vertep/custody/1.0#read`: the entries from a position on.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#read")]
    Read {
        /// The object.
        object: String,
        /// The first position to return (from 1).
        from: u64,
    },
    /// `https://w3id.org/vertep/custody/1.0#witness`: witness a `did:webvh` log entry of the
    /// object.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#witness")]
    Witness(WitnessRequest),
}

/// A request to witness the next entry of an object's `did:webvh` log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WitnessRequest {
    /// The object (its DID).
    pub object: String,
    /// The log the entry follows, as `did.jsonl` text; empty for the genesis entry.
    pub log: String,
    /// The log's witness proofs, as `did-witness.json` text.
    #[serde(default)]
    pub witness: Option<String>,
    /// The entry to witness.
    pub entry: serde_json::Value,
}

/// Equivocation evidence: two validly signed entries for one position and epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// The object.
    pub object: String,
    /// The epoch.
    pub epoch: u64,
    /// The position.
    pub position: u64,
    /// The writer that signed both.
    pub writer: String,
    /// The entry held first.
    pub first: WireEntry,
    /// The conflicting entry.
    pub second: WireEntry,
}

/// A custodian's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CustodyResponse {
    /// `promise`.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#promise")]
    Promise(EpochPromise),
    /// `refuse`: a higher epoch was promised, the epoch is refused after equivocation, or the
    /// batch carries an outdated configuration.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#refuse")]
    Refused {
        /// The highest epoch promised.
        highest: u64,
        /// Why.
        reason: String,
    },
    /// `ack`: the batch is stored; the custodian's head after it.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#ack")]
    Ack {
        /// The epoch accepted.
        epoch: u64,
        /// The head.
        head: u64,
    },
    /// The custodian's journal ends before the batch: resend from `head`.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#behind")]
    Behind {
        /// The head.
        head: u64,
    },
    /// The custodian holds a different entry of the same epoch at `position`.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#diverged")]
    Diverged {
        /// The position.
        position: u64,
        /// True when the conflict was recorded as equivocation evidence.
        evidence: bool,
    },
    /// `entries`: the answer to a read.
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#entries")]
    Entries {
        /// The entries from the requested position.
        entries: Vec<WireEntry>,
        /// Every content key wrap the custodian holds.
        keys: Vec<KeyWrap>,
        /// The configurations it accepted, with the epoch each took effect in.
        configs: Vec<(u64, CustodyConfig)>,
        /// The pending configuration it accepted, if a change is in progress.
        #[serde(default)]
        next: Option<CustodyConfig>,
    },
    /// `witnessed`: a witness proof (a Data Integrity proof over the entry's `versionId`).
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#witnessed")]
    Witnessed {
        /// The custodian.
        custodian: String,
        /// The proof.
        proof: serde_json::Value,
    },
    /// The request failed (malformed, bad signature, invalid entries, storage).
    #[serde(rename = "https://w3id.org/vertep/custody/1.0#failed")]
    Failed {
        /// Why.
        reason: String,
    },
}

/// The bytes signed for a value: a domain tag and its JSON.
pub fn signing_bytes<T: Serialize>(domain: &str, value: &T) -> Vec<u8> {
    let mut v = domain.as_bytes().to_vec();
    v.push(b'\n');
    v.extend(serde_json::to_vec(value).unwrap_or_default());
    v
}

impl EpochPromise {
    fn unsigned(&self) -> EpochPromise {
        EpochPromise {
            signature: String::new(),
            ..self.clone()
        }
    }

    /// Signs the promise.
    ///
    /// # Errors
    ///
    /// The signer's error.
    pub fn sign(mut self, signer: &dyn EntrySigner) -> Result<Self, KeyError> {
        let sig = signer.sign(&signing_bytes(
            "vertep-custody-promise-v1",
            &self.unsigned(),
        ))?;
        self.signature = b64(&sig);
        Ok(self)
    }

    /// True when the signature verifies against one of `keys`.
    pub fn verify(&self, keys: &[PublicKey]) -> bool {
        let Ok(sig) = unb64(&self.signature) else {
            return false;
        };
        let input = signing_bytes("vertep-custody-promise-v1", &self.unsigned());
        keys.iter().any(|k| k.verify(&input, &sig))
    }

    /// The head hash.
    pub fn head_digest(&self) -> Option<Digest> {
        self.head_hash.as_deref().and_then(Digest::from_hex)
    }
}

impl AppendBatch {
    fn unsigned(&self) -> AppendBatch {
        AppendBatch {
            signature: String::new(),
            ..self.clone()
        }
    }

    /// Signs the batch.
    ///
    /// # Errors
    ///
    /// The signer's error.
    pub fn sign(mut self, signer: &dyn EntrySigner) -> Result<Self, KeyError> {
        let sig = signer.sign(&signing_bytes("vertep-custody-batch-v1", &self.unsigned()))?;
        self.signature = b64(&sig);
        Ok(self)
    }

    /// True when the signature verifies against one of `keys`.
    pub fn verify(&self, keys: &[PublicKey]) -> bool {
        let Ok(sig) = unb64(&self.signature) else {
            return false;
        };
        let input = signing_bytes("vertep-custody-batch-v1", &self.unsigned());
        keys.iter().any(|k| k.verify(&input, &sig))
    }
}

/// Where custody finds parties' public keys.
pub trait KeyDirectory: Send + Sync {
    /// The keys `did` signs with (assertion and authentication methods).
    fn verification_keys(&self, did: &str) -> Option<Vec<PublicKey>>;
    /// The key-agreement key content keys are wrapped to.
    fn agreement_key(&self, did: &str) -> Option<PublicKey>;
    /// The keys of `object`'s controller, which sign custody reconfigurations (the `did:webvh`
    /// update keys).
    fn controller_keys(&self, object: &str) -> Option<Vec<PublicKey>>;
}

#[derive(Clone, Debug, Default)]
struct Keys {
    verify: Vec<PublicKey>,
    agree: Option<PublicKey>,
    controller: Vec<PublicKey>,
}

/// A key directory filled by hand (tests, and deployments that pin keys).
#[derive(Debug, Default)]
pub struct StaticKeyDirectory {
    keys: RwLock<BTreeMap<String, Keys>>,
}

impl StaticKeyDirectory {
    /// An empty directory.
    pub fn new() -> Self {
        Self::default()
    }

    fn with_entry(&self, did: &str, f: impl FnOnce(&mut Keys)) {
        let mut g = self
            .keys
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(g.entry(did.to_owned()).or_default());
    }

    /// Adds a verification key of `did`.
    pub fn add_verification(&self, did: &str, key: PublicKey) {
        self.with_entry(did, |k| k.verify.push(key));
    }

    /// Sets the key-agreement key of `did`.
    pub fn set_agreement(&self, did: &str, key: PublicKey) {
        self.with_entry(did, |k| k.agree = Some(key));
    }

    /// Adds a controller key of `object`.
    pub fn add_controller(&self, object: &str, key: PublicKey) {
        self.with_entry(object, |k| k.controller.push(key));
    }

    fn get(&self, did: &str) -> Option<Keys> {
        self.keys
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(did)
            .cloned()
    }
}

impl KeyDirectory for StaticKeyDirectory {
    fn verification_keys(&self, did: &str) -> Option<Vec<PublicKey>> {
        self.get(did).map(|k| k.verify).filter(|v| !v.is_empty())
    }
    fn agreement_key(&self, did: &str) -> Option<PublicKey> {
        self.get(did).and_then(|k| k.agree)
    }
    fn controller_keys(&self, object: &str) -> Option<Vec<PublicKey>> {
        self.get(object)
            .map(|k| k.controller)
            .filter(|v| !v.is_empty())
    }
}

/// A key directory that resolves DID documents: assertion and authentication methods sign,
/// the first key-agreement method receives content keys. Controller keys are the assertion
/// methods of the document's `controller` DIDs as listed in the object's own verification
/// methods (for `did:webvh`, the update keys appear there as the controller's methods).
pub struct ResolverKeyDirectory {
    resolver: Arc<dyn DidResolver>,
}

impl fmt::Debug for ResolverKeyDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolverKeyDirectory")
            .finish_non_exhaustive()
    }
}

impl ResolverKeyDirectory {
    /// A directory over `resolver`.
    pub fn new(resolver: Arc<dyn DidResolver>) -> Self {
        ResolverKeyDirectory { resolver }
    }

    fn doc(&self, did: &str) -> Option<vertep_federation::DidDocument> {
        let did = Did::parse(did).ok()?;
        self.resolver.resolve(&did).ok().map(|r| r.document)
    }
}

impl KeyDirectory for ResolverKeyDirectory {
    fn verification_keys(&self, did: &str) -> Option<Vec<PublicKey>> {
        let doc = self.doc(did)?;
        let mut keys: Vec<PublicKey> = doc
            .assertion_methods()
            .into_iter()
            .chain(doc.authentication_methods())
            .map(|m| m.public_key)
            .collect();
        keys.dedup();
        (!keys.is_empty()).then_some(keys)
    }
    fn agreement_key(&self, did: &str) -> Option<PublicKey> {
        let doc = self.doc(did)?;
        doc.key_agreement_methods().first().map(|m| m.public_key)
    }
    fn controller_keys(&self, object: &str) -> Option<Vec<PublicKey>> {
        let doc = self.doc(object)?;
        let keys: Vec<PublicKey> = doc
            .verification_method
            .iter()
            .filter(|m| m.controller != doc.id)
            .map(|m| m.public_key)
            .collect();
        (!keys.is_empty()).then_some(keys)
    }
}

/// The object IRI of a message's `object` field.
///
/// # Errors
///
/// [`WireError`] when it is not an IRI.
pub fn object_iri(object: &str) -> Result<NamedNode, WireError> {
    NamedNode::new(object).map_err(|e| WireError(format!("object {object}: {e}")))
}
