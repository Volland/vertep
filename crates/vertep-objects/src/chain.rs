//! Verifiable journal entries: each entry of a durable object carries its custody link (object,
//! position, epoch, writer, previous hash), its own content hash and the writer's signature,
//! all as `vt:` statements inside the entry graph.
//!
//! Keeping the link in the graph means every journal store round-trips it untouched: stores only
//! add their stamps (`rdf:type vt:JournalEntry`, `vt:actor`, `vt:position`), and this module
//! writes `vt:actor` and `vt:position` itself with the values the store would stamp.
//!
//! The content hash is SHA-256 over the canonical form of the entry graph without the
//! `vt:entryHash` and `vt:entrySignature` statements and the `rdf:type vt:JournalEntry` stamp.
//! The canonical form is the sorted set of N-Triples lines; an entry with blank nodes is first
//! relabelled with RDFC-1.0, so a store that renames blank nodes does not change the hash.
//! Runtime entries have no blank nodes, so the common path is a sort.

use core::fmt;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use oxrdf::dataset::CanonicalizationAlgorithm;
use oxrdf::vocab::xsd;
use oxrdf::{Graph, Literal, NamedNode, NamedOrBlankNode, Term, TermRef, Triple};
use sha2::{Digest as _, Sha256};
use vertep_federation::{KeyError, KeyManager, PublicKey};
use vertep_store::{Entry, JournalRecord, Position, vocab as store_vocab};

use crate::vocab;

/// A SHA-256 digest of an entry's canonical form.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    /// The lowercase hexadecimal form.
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in self.0 {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Parses 64 hexadecimal digits.
    pub fn from_hex(s: &str) -> Option<Self> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hi = (chunk[0] as char).to_digit(16)?;
            let lo = (chunk[1] as char).to_digit(16)?;
            out[i] = (hi * 16 + lo) as u8;
        }
        Some(Digest(out))
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({})", &self.to_hex()[..12])
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// The custody link of one entry: what the writer states about where the entry belongs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainLink {
    /// The durable object's address (its DID as an IRI).
    pub object: NamedNode,
    /// The entry's position in the object's journal.
    pub position: Position,
    /// The custody epoch the writer holds.
    pub epoch: u64,
    /// The writing host's DID.
    pub writer: String,
    /// The hash of the entry at `position - 1`; `None` only at position 1.
    pub prev: Option<Digest>,
}

/// The custody link and seal read back from an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedLink {
    /// The link the writer stated.
    pub link: ChainLink,
    /// The content hash stated in the entry.
    pub hash: Digest,
    /// The writer's signature over [`signing_input`] of `hash`.
    pub signature: Vec<u8>,
}

/// Why sealing or reading an entry failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// The entry lacks a link statement or states one twice or malformed.
    Malformed(String),
    /// The entry already states an actor or position different from the link.
    Conflict(String),
    /// The signer failed.
    Signing(KeyError),
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChainError::Malformed(m) => write!(f, "malformed custody link: {m}"),
            ChainError::Conflict(m) => write!(f, "custody link conflict: {m}"),
            ChainError::Signing(e) => write!(f, "signing failed: {e}"),
        }
    }
}

impl std::error::Error for ChainError {}

/// Signs entry hashes on behalf of a host.
pub trait EntrySigner: Send + Sync {
    /// The host's DID, written as `vt:writer`.
    fn writer(&self) -> &str;
    /// A signature over `data`.
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the key manager cannot sign.
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, KeyError>;
}

/// An [`EntrySigner`] that signs with a named Ed25519 key held by a [`KeyManager`].
#[derive(Clone)]
pub struct KeyManagerSigner {
    did: String,
    keys: Arc<dyn KeyManager>,
    key: String,
}

impl fmt::Debug for KeyManagerSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyManagerSigner")
            .field("did", &self.did)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl KeyManagerSigner {
    /// Signs as `did` with key `key` of `keys`.
    pub fn new(did: impl Into<String>, keys: Arc<dyn KeyManager>, key: impl Into<String>) -> Self {
        KeyManagerSigner {
            did: did.into(),
            keys,
            key: key.into(),
        }
    }

    /// The public key the signatures verify against.
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the key is unknown.
    pub fn public_key(&self) -> Result<PublicKey, KeyError> {
        self.keys.public_key(&self.key)
    }
}

impl EntrySigner for KeyManagerSigner {
    fn writer(&self) -> &str {
        &self.did
    }
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.keys.sign(&self.key, data)
    }
}

/// The bytes a writer signs for an entry with content hash `hash`: a domain tag and the hash.
pub fn signing_input(hash: &Digest) -> Vec<u8> {
    let mut v = b"vertep-entry-v1\n".to_vec();
    v.extend_from_slice(&hash.0);
    v
}

fn is_seal_statement(entry: &NamedNode, t: &Triple) -> bool {
    if t.subject != NamedOrBlankNode::from(entry.clone()) {
        return false;
    }
    let p = t.predicate.as_str();
    p == vocab::ENTRY_HASH
        || p == vocab::ENTRY_SIGNATURE
        || (p == store_vocab::RDF_TYPE
            && t.object == Term::from(store_vocab::node(store_vocab::JOURNAL_ENTRY)))
}

/// The content hash of `entry`: SHA-256 of the canonical form of its graph without the seal
/// statements and the `rdf:type vt:JournalEntry` stamp.
pub fn content_hash(entry: &Entry) -> Digest {
    let mut g = Graph::new();
    for t in entry.triples() {
        let t = t.into_owned();
        if !is_seal_statement(entry.name(), &t) {
            g.insert(&t);
        }
    }
    let has_blank = g.iter().any(|t| {
        matches!(t.subject, oxrdf::NamedOrBlankNodeRef::BlankNode(_))
            || matches!(t.object, TermRef::BlankNode(_))
    });
    if has_blank {
        g.canonicalize(CanonicalizationAlgorithm::default());
    }
    let mut lines: Vec<String> = g.iter().map(|t| format!("{t} .")).collect();
    lines.sort_unstable();
    let mut h = Sha256::new();
    h.update(entry.name().as_str().as_bytes());
    h.update(b"\n");
    for l in &lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    Digest(h.finalize().into())
}

fn int(n: u64) -> Literal {
    Literal::new_typed_literal(n.to_string(), xsd::INTEGER)
}

// @tg: implements:: [[openspec:durable-objects#Verifiable Journal Entries]]
/// `entry` with its custody link, content hash and the signer's signature added.
///
/// # Errors
///
/// [`ChainError::Conflict`] when the entry already states another actor or position, or any
/// link statement; [`ChainError::Signing`] when signing fails; [`ChainError::Malformed`] when
/// the writer is not an IRI.
pub fn seal(
    entry: &Entry,
    link: &ChainLink,
    signer: &dyn EntrySigner,
) -> Result<Entry, ChainError> {
    if signer.writer() != link.writer {
        return Err(ChainError::Conflict(format!(
            "link names writer {}, signer is {}",
            link.writer,
            signer.writer()
        )));
    }
    let subject = NamedOrBlankNode::from(entry.name().clone());
    for t in entry.triples() {
        if t.subject != subject.as_ref() {
            continue;
        }
        let p = t.predicate.as_str();
        if [
            vocab::CUSTODY_EPOCH,
            vocab::WRITER,
            vocab::PREVIOUS_ENTRY_HASH,
            vocab::ENTRY_HASH,
            vocab::ENTRY_SIGNATURE,
        ]
        .contains(&p)
        {
            return Err(ChainError::Conflict(format!(
                "entry {} is already sealed ({p})",
                entry.name()
            )));
        }
    }
    // The store's own stamp check rejects a different actor or position; checking here too
    // keeps the error in custody terms.
    let stamped = entry
        .stamp(&link.object, link.position)
        .map_err(|e| ChainError::Conflict(e.to_string()))?;
    let writer = NamedNode::new(link.writer.clone())
        .map_err(|e| ChainError::Malformed(format!("writer {}: {e}", link.writer)))?;
    let mut triples: Vec<Triple> = stamped
        .triples()
        .iter()
        .map(|t| t.into_owned())
        .filter(|t| !is_seal_statement(entry.name(), t))
        .collect();
    triples.push(Triple::new(
        subject.clone(),
        vocab::node(vocab::CUSTODY_EPOCH),
        int(link.epoch),
    ));
    triples.push(Triple::new(
        subject.clone(),
        vocab::node(vocab::WRITER),
        writer,
    ));
    if let Some(prev) = link.prev {
        triples.push(Triple::new(
            subject.clone(),
            vocab::node(vocab::PREVIOUS_ENTRY_HASH),
            Literal::new_simple_literal(prev.to_hex()),
        ));
    }
    let unsigned = Entry::new(entry.name().clone(), triples.clone());
    let hash = content_hash(&unsigned);
    let signature = signer
        .sign(&signing_input(&hash))
        .map_err(ChainError::Signing)?;
    triples.push(Triple::new(
        subject.clone(),
        vocab::node(vocab::ENTRY_HASH),
        Literal::new_simple_literal(hash.to_hex()),
    ));
    triples.push(Triple::new(
        subject,
        vocab::node(vocab::ENTRY_SIGNATURE),
        Literal::new_simple_literal(URL_SAFE_NO_PAD.encode(signature)),
    ));
    Ok(Entry::new(entry.name().clone(), triples))
}

fn one<'a>(entry: &'a Entry, predicate: &str) -> Result<Option<TermRef<'a>>, ChainError> {
    let p = vocab::node(predicate);
    let mut it = entry
        .triples()
        .objects_for_subject_predicate(entry.name(), &p);
    let first = it.next();
    if it.next().is_some() {
        return Err(ChainError::Malformed(format!(
            "{} states {predicate} twice",
            entry.name()
        )));
    }
    Ok(first)
}

fn literal_str<'a>(t: TermRef<'a>, what: &str) -> Result<&'a str, ChainError> {
    match t {
        TermRef::Literal(l) => Ok(l.value()),
        _ => Err(ChainError::Malformed(format!("{what} is not a literal"))),
    }
}

fn required<'a>(entry: &'a Entry, predicate: &str) -> Result<TermRef<'a>, ChainError> {
    one(entry, predicate)?
        .ok_or_else(|| ChainError::Malformed(format!("{} lacks {predicate}", entry.name())))
}

/// The custody link and seal stated in `entry`.
///
/// # Errors
///
/// [`ChainError::Malformed`] when a statement is missing, repeated or ill-formed.
pub fn read_link(entry: &Entry) -> Result<SealedLink, ChainError> {
    let object = match required(entry, store_vocab::ACTOR)? {
        TermRef::NamedNode(n) => n.into_owned(),
        _ => return Err(ChainError::Malformed("vt:actor is not an IRI".into())),
    };
    let position = entry
        .position()
        .ok_or_else(|| ChainError::Malformed(format!("{} lacks vt:position", entry.name())))?;
    let epoch = literal_str(required(entry, vocab::CUSTODY_EPOCH)?, "vt:custodyEpoch")?
        .parse::<u64>()
        .map_err(|e| ChainError::Malformed(format!("vt:custodyEpoch: {e}")))?;
    let writer = match required(entry, vocab::WRITER)? {
        TermRef::NamedNode(n) => n.as_str().to_owned(),
        _ => return Err(ChainError::Malformed("vt:writer is not an IRI".into())),
    };
    let prev = match one(entry, vocab::PREVIOUS_ENTRY_HASH)? {
        None => None,
        Some(t) => Some(
            Digest::from_hex(literal_str(t, "vt:previousEntryHash")?)
                .ok_or_else(|| ChainError::Malformed("vt:previousEntryHash".into()))?,
        ),
    };
    let hash = Digest::from_hex(literal_str(
        required(entry, vocab::ENTRY_HASH)?,
        "vt:entryHash",
    )?)
    .ok_or_else(|| ChainError::Malformed("vt:entryHash".into()))?;
    let signature = URL_SAFE_NO_PAD
        .decode(literal_str(
            required(entry, vocab::ENTRY_SIGNATURE)?,
            "vt:entrySignature",
        )?)
        .map_err(|e| ChainError::Malformed(format!("vt:entrySignature: {e}")))?;
    Ok(SealedLink {
        link: ChainLink {
            object,
            position,
            epoch,
            writer,
            prev,
        },
        hash,
        signature,
    })
}

/// Who may write in an epoch, and with which keys.
pub trait HostKeys {
    /// The verification keys of `writer` if it was a host of `object` in `epoch`; `None` when
    /// it was not.
    fn host_keys(&self, object: &NamedNode, epoch: u64, writer: &str) -> Option<Vec<PublicKey>>;
}

/// Why a replica failed verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FailureReason {
    /// The stated or chained hash does not match the content.
    HashMismatch,
    /// The signature does not verify against the writer's keys.
    BadSignature,
    /// The signer was not a host of the entry's epoch.
    NotAHost,
    /// An epoch lower than the previous entry's.
    EpochRegression,
    /// The entry is not a sealed entry of this object at this position.
    Malformed(String),
}

impl FailureReason {
    /// The short reason code: `hash-mismatch`, `bad-signature`, `not-a-host`,
    /// `epoch-regression` or `malformed`.
    pub fn code(&self) -> &'static str {
        match self {
            FailureReason::HashMismatch => "hash-mismatch",
            FailureReason::BadSignature => "bad-signature",
            FailureReason::NotAHost => "not-a-host",
            FailureReason::EpochRegression => "epoch-regression",
            FailureReason::Malformed(_) => "malformed",
        }
    }
}

/// The first position at which a replica failed verification, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyFailure {
    /// The failing entry's position.
    pub position: Position,
    /// The reason.
    pub reason: FailureReason,
}

impl fmt::Display for VerifyFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "entry {}: {}", self.position.get(), self.reason.code())?;
        if let FailureReason::Malformed(m) = &self.reason {
            write!(f, " ({m})")?;
        }
        Ok(())
    }
}

impl std::error::Error for VerifyFailure {}

/// The verified state at the end of a chain: the last entry's position, hash and epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainHead {
    /// The last position (0 for an empty chain).
    pub position: Position,
    /// The last entry's hash; `None` for an empty chain.
    pub hash: Option<Digest>,
    /// The last entry's epoch (0 for an empty chain).
    pub epoch: u64,
}

impl ChainHead {
    /// The empty chain.
    pub const EMPTY: ChainHead = ChainHead {
        position: Position::ZERO,
        hash: None,
        epoch: 0,
    };
}

/// Verifies one sealed entry that follows `head`.
///
/// # Errors
///
/// The [`VerifyFailure`] for this entry.
pub fn verify_entry(
    object: &NamedNode,
    head: ChainHead,
    entry: &Entry,
    keys: &dyn HostKeys,
) -> Result<ChainHead, VerifyFailure> {
    let position = head.position.next();
    let fail = |reason| VerifyFailure { position, reason };
    let sealed = read_link(entry).map_err(|e| fail(FailureReason::Malformed(e.to_string())))?;
    if &sealed.link.object != object || sealed.link.position != position {
        return Err(fail(FailureReason::Malformed(format!(
            "states {} at {}",
            sealed.link.object,
            sealed.link.position.get()
        ))));
    }
    if content_hash(entry) != sealed.hash || sealed.link.prev != head.hash {
        return Err(fail(FailureReason::HashMismatch));
    }
    if sealed.link.epoch < head.epoch {
        return Err(fail(FailureReason::EpochRegression));
    }
    let Some(public) = keys.host_keys(object, sealed.link.epoch, &sealed.link.writer) else {
        return Err(fail(FailureReason::NotAHost));
    };
    let input = signing_input(&sealed.hash);
    if !public.iter().any(|k| k.verify(&input, &sealed.signature)) {
        return Err(fail(FailureReason::BadSignature));
    }
    Ok(ChainHead {
        position,
        hash: Some(sealed.hash),
        epoch: sealed.link.epoch,
    })
}

// @tg: implements:: [[openspec:durable-objects#Verifiable Journal Entries]]
/// Verifies `records` as the continuation of `from`: every signature, every previous-hash
/// link, positions without gaps, epochs that never decrease and that each signer was a host of
/// its entry's epoch. Returns the verified head.
///
/// # Errors
///
/// The first [`VerifyFailure`].
pub fn verify_chain(
    object: &NamedNode,
    from: ChainHead,
    records: &[JournalRecord],
    keys: &dyn HostKeys,
) -> Result<ChainHead, VerifyFailure> {
    let mut head = from;
    for r in records {
        if r.position != head.position.next() {
            return Err(VerifyFailure {
                position: r.position,
                reason: FailureReason::Malformed(format!(
                    "expected position {}",
                    head.position.next().get()
                )),
            });
        }
        head = verify_entry(object, head, &r.entry, keys)?;
    }
    Ok(head)
}
