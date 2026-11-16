//! Sealed sensitive fields and erasure by key destruction
//! ([[openspec:journal-redaction#Sensitive Fields]], [[openspec:journal-redaction#Erasure By Key Destruction]]).
//!
//! [`SealingJournal`] wraps any [`AgentJournal`]. On append it replaces each field a
//! [`SealPolicy`] marks sensitive by a ciphertext under the data key of the field's subject, so
//! the stored entry, and therefore any hash or signature over it, covers only ciphertext. On
//! read it opens what the key store can open and puts an erased marker where it cannot. Erasure
//! destroys the subject's key through the [`KeyStore`] and appends an `erasure` entry to every
//! journal that held the subject's data; no entry is rewritten, so the chain still verifies.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use vertep_core::datamodel::Value;

use crate::journal::{AgentInput, AgentJournal, JournalEntry, JournalError};

/// The key under which a sealed field is stored in place of its value.
pub const SEALED: &str = "$sealed";
/// The key under which an unreadable (erased) field is shown in place of its value.
pub const ERASED: &str = "$erased";

/// Holds the per-subject data keys. A conforming store destroys a key for good: after
/// [`KeyStore::destroy`] nothing can read the key again, from this store or a copy of it.
pub trait KeyStore: Send + Sync {
    /// The subject's key, when it exists and was not destroyed.
    fn key(&self, subject: &str) -> Option<[u8; 32]>;
    /// The subject's key, created on first use. Fails for an erased subject: an erased subject
    /// never gets a new key.
    fn obtain(&self, subject: &str) -> Result<[u8; 32], String>;
    /// Destroys the key and remembers that the subject is erased.
    fn destroy(&self, subject: &str) -> Result<(), String>;
    /// Whether [`KeyStore::destroy`] guarantees the key cannot be recovered. A store that cannot
    /// say yes (a backup that keeps old keys, say) is refused by [`SealingJournal::new`].
    fn destruction_guaranteed(&self) -> bool;
}

#[derive(Debug, Default)]
struct Keys {
    live: BTreeMap<String, [u8; 32]>,
    erased: BTreeSet<String>,
}

/// A key store in memory, for tests and short-lived hosts.
#[derive(Debug, Default)]
pub struct MemoryKeyStore {
    inner: Mutex<Keys>,
}

impl MemoryKeyStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for MemoryKeyStore {
    fn key(&self, subject: &str) -> Option<[u8; 32]> {
        self.inner.lock().ok()?.live.get(subject).copied()
    }

    fn obtain(&self, subject: &str) -> Result<[u8; 32], String> {
        let mut g = self.inner.lock().map_err(|_| "poisoned".to_owned())?;
        if g.erased.contains(subject) {
            return Err(format!("subject {subject} is erased"));
        }
        Ok(*g
            .live
            .entry(subject.to_owned())
            .or_insert_with(|| ChaCha20Poly1305::generate_key(&mut OsRng).into()))
    }

    fn destroy(&self, subject: &str) -> Result<(), String> {
        let mut g = self.inner.lock().map_err(|_| "poisoned".to_owned())?;
        if let Some(mut k) = g.live.remove(subject) {
            k.fill(0);
        }
        g.erased.insert(subject.to_owned());
        Ok(())
    }

    fn destruction_guaranteed(&self) -> bool {
        true
    }
}

/// One field a chart type marks sensitive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensitiveField {
    /// The event name, or the chosen event of a decision.
    pub event: String,
    /// A JSON pointer to the field in the event data (or decision payload).
    pub field: String,
    /// A JSON pointer, in the same data, to the string naming the data subject.
    pub subject: String,
    /// How long after the entry the field may be kept; once past, the subject is erased with
    /// the reason `retention`. None keeps it until an explicit erasure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_ms: Option<i64>,
}

/// The fields to seal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealPolicy {
    /// The marked fields.
    pub fields: Vec<SensitiveField>,
}

/// What a sealed or erased journal can say about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Erasures {
    /// Subjects whose data is unreadable.
    pub subjects: BTreeSet<String>,
}

impl Erasures {
    /// Whether any data is unreadable, so a replay can only follow what was recorded.
    pub fn is_empty(&self) -> bool {
        self.subjects.is_empty()
    }
}

/// An agent journal that seals sensitive fields and can erase subjects.
pub struct SealingJournal {
    inner: Arc<dyn AgentJournal>,
    keys: Arc<dyn KeyStore>,
    policy: SealPolicy,
}

impl core::fmt::Debug for SealingJournal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SealingJournal")
            .field("fields", &self.policy.fields.len())
            .finish_non_exhaustive()
    }
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn aad(actor: &str, position: u64, field: &str) -> Vec<u8> {
    format!("{actor}\u{0}{position}\u{0}{field}").into_bytes()
}

/// Runs `f` over the JSON of the data an input carries (an event's data or a decision's
/// payload) and the event name it is filed under, and stores the data back.
fn with_data<R>(input: &mut AgentInput, f: impl FnOnce(&str, &mut Json) -> R) -> Option<R> {
    let (name, value): (String, &mut Value) = match input {
        AgentInput::Event { event, .. } => (event.name.clone(), &mut event.data),
        AgentInput::Decision { event, payload, .. } => (event.clone(), payload),
        _ => return None,
    };
    let mut json = crate::json::to_json(value);
    let out = f(&name, &mut json);
    *value = crate::json::from_json(&json);
    Some(out)
}

impl SealingJournal {
    /// Wraps `inner`. Refuses a key store that does not guarantee destruction.
    pub fn new(
        inner: Arc<dyn AgentJournal>,
        keys: Arc<dyn KeyStore>,
        policy: SealPolicy,
    ) -> Result<Self, String> {
        if !keys.destruction_guaranteed() {
            return Err("the key store does not guarantee key destruction".into());
        }
        Ok(SealingJournal {
            inner,
            keys,
            policy,
        })
    }

    fn seal(&self, actor: &str, entry: &mut JournalEntry) -> Result<(), JournalError> {
        let (position, ts) = (entry.position, entry.ts);
        with_data(&mut entry.input, |event, data| {
            for f in self.policy.fields.iter().filter(|f| f.event == event) {
                let Some(subject) = data
                    .pointer(&f.subject)
                    .and_then(Json::as_str)
                    .map(str::to_owned)
                else {
                    continue;
                };
                let Some(slot) = data.pointer_mut(&f.field) else {
                    continue;
                };
                if slot.get(SEALED).is_some() {
                    continue;
                }
                let key = self.keys.obtain(&subject).map_err(JournalError::Backend)?;
                let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
                let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
                let plain =
                    serde_json::to_vec(slot).map_err(|e| JournalError::Backend(e.to_string()))?;
                let ct = cipher
                    .encrypt(
                        &nonce,
                        Payload {
                            msg: &plain,
                            aad: &aad(actor, position, &f.field),
                        },
                    )
                    .map_err(|e| JournalError::Backend(e.to_string()))?;
                let mut sealed = json!({
                    "subject": subject,
                    "nonce": hex(&nonce),
                    "ct": hex(&ct),
                });
                if let Some(r) = f.retention_ms {
                    sealed["until"] = json!(ts.saturating_add(r));
                }
                *slot = json!({ SEALED: sealed });
            }
            Ok(())
        })
        .unwrap_or(Ok(()))
    }

    /// Opens every sealed field of `entry` the key store can open; the others become
    /// `{"$erased": <subject>}`. Returns the subjects that could not be opened.
    fn open(&self, actor: &str, entry: &mut JournalEntry) -> BTreeSet<String> {
        let position = entry.position;
        with_data(&mut entry.input, |_, data| {
            let mut gone = BTreeSet::new();
            let mut paths = Vec::new();
            find_sealed(data, String::new(), &mut paths);
            for path in paths {
                let Some(slot) = data.pointer_mut(&path) else {
                    continue;
                };
                let sealed = slot[SEALED].clone();
                let subject = sealed["subject"].as_str().unwrap_or_default().to_owned();
                let opened = (|| {
                    let key = self.keys.key(&subject)?;
                    let nonce = unhex(sealed["nonce"].as_str()?)?;
                    let ct = unhex(sealed["ct"].as_str()?)?;
                    if nonce.len() != 12 {
                        return None;
                    }
                    let plain = ChaCha20Poly1305::new(Key::from_slice(&key))
                        .decrypt(
                            Nonce::from_slice(&nonce),
                            Payload {
                                msg: &ct,
                                aad: &aad(actor, position, &path),
                            },
                        )
                        .ok()?;
                    serde_json::from_slice::<Json>(&plain).ok()
                })();
                match opened {
                    Some(v) => *slot = v,
                    None => {
                        *slot = json!({ ERASED: subject });
                        gone.insert(subject);
                    }
                }
            }
            gone
        })
        .unwrap_or_default()
    }

    /// The subjects whose data in `actor`'s journal can no longer be read.
    pub fn erasures(&self, actor: &str) -> Erasures {
        let mut subjects = BTreeSet::new();
        for mut e in self.inner.read(actor) {
            subjects.extend(self.open(actor, &mut e));
        }
        Erasures { subjects }
    }

    /// Destroys `subject`'s key and appends an `erasure` entry, with `reason`, to every
    /// journal that holds the subject's sealed data. Returns the actors it appended to.
    ///
    /// The key is destroyed first: if an append then fails the data is already gone and the
    /// call can be repeated to record it.
    // @tg: implements:: [[openspec:journal-redaction#Erasure By Key Destruction]]
    pub fn erase(&self, subject: &str, reason: &str, ts: i64) -> Result<Vec<String>, JournalError> {
        let holders: Vec<String> = self
            .inner
            .actors()
            .into_iter()
            .filter(|a| {
                self.inner
                    .read(a)
                    .iter()
                    .any(|e| mentions(&e.input, subject))
            })
            .collect();
        self.keys.destroy(subject).map_err(JournalError::Backend)?;
        for actor in &holders {
            let head = self.inner.read(actor).last().map_or(0, |e| e.position);
            if self
                .inner
                .read(actor)
                .iter()
                .any(|e| matches!(&e.input, AgentInput::Erasure { subject: s, .. } if s == subject))
            {
                continue;
            }
            self.inner.append(
                actor,
                JournalEntry {
                    position: head + 1,
                    ts,
                    input: AgentInput::Erasure {
                        subject: subject.to_owned(),
                        reason: reason.to_owned(),
                        retracted: Vec::new(),
                    },
                    disposition: None,
                    offered: None,
                },
            )?;
        }
        Ok(holders)
    }

    /// Erases every subject with a sealed field whose retention horizon passed by `now_ms`,
    /// with the reason `retention`. A chart type without a horizon never expires.
    // @tg: implements:: [[openspec:journal-redaction#Retention Policy]]
    pub fn expire(&self, now_ms: i64) -> Result<Vec<String>, JournalError> {
        let mut due = BTreeSet::new();
        for actor in self.inner.actors() {
            for e in self.inner.read(&actor) {
                collect_due(&e.input, now_ms, &mut due);
            }
        }
        let mut erased = Vec::new();
        for subject in due {
            if self.keys.key(&subject).is_some() {
                self.erase(&subject, "retention", now_ms)?;
                erased.push(subject);
            }
        }
        Ok(erased)
    }
}

/// Whether the input carries a value that was erased (its key is gone), so a replay of it can
/// only follow what was recorded.
pub fn has_erased(input: &AgentInput) -> bool {
    let mut input = input.clone();
    with_data(&mut input, |_, data| contains_key(data, ERASED)).unwrap_or(false)
}

fn contains_key(v: &Json, key: &str) -> bool {
    match v {
        Json::Object(m) => m.contains_key(key) || m.values().any(|c| contains_key(c, key)),
        Json::Array(a) => a.iter().any(|c| contains_key(c, key)),
        _ => false,
    }
}

fn find_sealed(v: &Json, at: String, out: &mut Vec<String>) {
    match v {
        Json::Object(m) if m.contains_key(SEALED) => out.push(at),
        Json::Object(m) => m.iter().for_each(|(k, c)| {
            find_sealed(
                c,
                format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")),
                out,
            )
        }),
        Json::Array(a) => a
            .iter()
            .enumerate()
            .for_each(|(i, c)| find_sealed(c, format!("{at}/{i}"), out)),
        _ => {}
    }
}

fn sealed_markers(input: &AgentInput) -> Vec<Json> {
    let mut input = input.clone();
    with_data(&mut input, |_, data| {
        let mut paths = Vec::new();
        find_sealed(data, String::new(), &mut paths);
        paths
            .iter()
            .filter_map(|p| data.pointer(p).map(|s| s[SEALED].clone()))
            .collect()
    })
    .unwrap_or_default()
}

fn mentions(input: &AgentInput, subject: &str) -> bool {
    sealed_markers(input)
        .iter()
        .any(|m| m["subject"].as_str() == Some(subject))
}

fn collect_due(input: &AgentInput, now_ms: i64, due: &mut BTreeSet<String>) {
    for m in sealed_markers(input) {
        if let (Some(until), Some(s)) = (m["until"].as_i64(), m["subject"].as_str())
            && until <= now_ms
        {
            due.insert(s.to_owned());
        }
    }
}

impl AgentJournal for SealingJournal {
    // @tg: implements:: [[openspec:journal-redaction#Sensitive Fields]]
    fn append(&self, actor: &str, mut entry: JournalEntry) -> Result<(), JournalError> {
        self.seal(actor, &mut entry)?;
        self.inner.append(actor, entry)
    }

    fn read(&self, actor: &str) -> Vec<JournalEntry> {
        self.try_read(actor).unwrap_or_default()
    }

    fn try_read(&self, actor: &str) -> Result<Vec<JournalEntry>, JournalError> {
        let mut entries = self.inner.try_read(actor)?;
        for e in &mut entries {
            self.open(actor, e);
        }
        Ok(entries)
    }

    fn actors(&self) -> Vec<String> {
        self.inner.actors()
    }
}
