//! Writing `did:webvh` logs: create, update, rotate and deactivate, signed through the key
//! manager.
//!
//! An [`Author`] holds the names of the controller's update keys ([`UpdateKeys`]), never the
//! keys. With pre-rotation, every entry reveals the key the previous entry announced (by hash)
//! and announces a fresh one, so a stolen current key cannot extend the log.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Timelike, Utc};
use didwebvh_rs::DIDWebVHState;
use didwebvh_rs::Multibase;
use didwebvh_rs::log_entry::LogEntryMethods as _;
use didwebvh_rs::parameters::Parameters;
use didwebvh_rs::witness::{Witness, Witnesses};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vertep_federation::{KeyManager, KeyType};

use crate::error::WebvhError;
use crate::log::DidLog;
use crate::signer::{KmSigner, key_hash};

/// The time source for `versionTime`. The default is the system clock; entries made within one
/// second then wait for the next second, since `versionTime` must strictly increase.
pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// The controller's update keys, by name in the key manager.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateKeys {
    /// The key that signs the next entry when pre-rotation is off.
    pub current: String,
    /// With pre-rotation: the key whose hash the last entry announced, which signs the next.
    pub next: Option<String>,
}

/// A witness list: `did:key` DIDs and how many proofs an entry needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witnessing {
    /// The witnesses' `did:key` DIDs.
    pub witnesses: Vec<String>,
    /// The number of proofs an entry needs.
    pub threshold: u32,
}

impl Witnessing {
    fn parameter(&self) -> Result<Arc<Witnesses>, WebvhError> {
        let w = Witnesses::Value {
            threshold: self.threshold,
            witnesses: self.witnesses.iter().map(Witness::new).collect(),
        };
        w.validate()
            .map_err(|e| WebvhError::Refused(format!("witness list: {e}")))?;
        Ok(Arc::new(w))
    }
}

/// Where and how to create a DID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateOptions {
    /// The domain, with an optional port (`example.org`, `example.org:8443`).
    pub domain: String,
    /// The path segments (`["objects", "ticket-8812"]`); empty for `/.well-known/`.
    pub path: Vec<String>,
    /// Announce the next update key by hash (pre-rotation).
    pub pre_rotation: bool,
    /// The witness list, if entries need witness proofs.
    pub witnesses: Option<Witnessing>,
    /// The `ttl` parameter in seconds.
    pub ttl: Option<u32>,
}

impl CreateOptions {
    /// A DID at `domain`, without a path, pre-rotation or witnesses.
    pub fn new(domain: impl Into<String>) -> Self {
        CreateOptions {
            domain: domain.into(),
            path: Vec::new(),
            pre_rotation: false,
            witnesses: None,
            ttl: None,
        }
    }

    /// Sets the path from `a:b` or `a/b`.
    #[must_use]
    pub fn path(mut self, path: &str) -> Self {
        self.path = path
            .split([':', '/'])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        self
    }

    /// Turns pre-rotation on or off.
    #[must_use]
    pub fn pre_rotation(mut self, on: bool) -> Self {
        self.pre_rotation = on;
        self
    }

    /// Requires witness proofs from `witnesses`.
    #[must_use]
    pub fn witnesses(mut self, witnesses: Witnessing) -> Self {
        self.witnesses = Some(witnesses);
        self
    }

    /// Sets the `ttl` parameter.
    #[must_use]
    pub fn ttl(mut self, ttl: u32) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// The DID with the `{SCID}` placeholder that the document must use as its `id` (and in
    /// its verification method and service ids); creation replaces the placeholder.
    pub fn template_did(&self) -> String {
        let mut s = format!("did:webvh:{{SCID}}:{}", self.domain.replace(':', "%3A"));
        for p in &self.path {
            s.push(':');
            s.push_str(p);
        }
        s
    }
}

/// One new log entry: what changes. Unset fields stay as they are.
#[derive(Clone, Debug, Default)]
pub struct EntryChange {
    /// The new document.
    pub document: Option<Value>,
    /// A new witness list (`Some(None)` turns witnessing off).
    pub witnesses: Option<Option<Witnessing>>,
    /// A new `ttl`.
    pub ttl: Option<u32>,
    /// Rotate to this key (by name). With pre-rotation it must be the announced key.
    pub rotate_to: Option<String>,
    /// Turn pre-rotation off with this entry.
    pub end_pre_rotation: bool,
    /// Deactivate the DID.
    pub deactivate: bool,
}

/// Writes the log of one DID as its controller.
pub struct Author {
    keys: Arc<dyn KeyManager>,
    update: UpdateKeys,
    clock: Option<Clock>,
}

impl std::fmt::Debug for Author {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Author")
            .field("update", &self.update)
            .finish_non_exhaustive()
    }
}

fn floor(t: DateTime<Utc>) -> DateTime<Utc> {
    t.with_nanosecond(0).unwrap_or(t)
}

impl Author {
    /// The controller with update keys `update` in `keys`.
    pub fn new(keys: Arc<dyn KeyManager>, update: UpdateKeys) -> Self {
        Author {
            keys,
            update,
            clock: None,
        }
    }

    /// A controller with a new update key (and a new announced next key with pre-rotation).
    ///
    /// # Errors
    ///
    /// [`WebvhError::Keys`] when the key manager cannot create keys.
    pub fn generate(keys: Arc<dyn KeyManager>, pre_rotation: bool) -> Result<Self, WebvhError> {
        let current = keys.generate(KeyType::Ed25519)?;
        let next = if pre_rotation {
            Some(keys.generate(KeyType::Ed25519)?)
        } else {
            None
        };
        Ok(Author::new(keys, UpdateKeys { current, next }))
    }

    /// Takes `versionTime` from `clock` instead of the system clock. Times must not be in the
    /// future; a time not after the last entry becomes the last entry's time plus one second.
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }

    /// The update keys.
    pub fn update_keys(&self) -> &UpdateKeys {
        &self.update
    }

    /// The key manager.
    pub fn keys(&self) -> &Arc<dyn KeyManager> {
        &self.keys
    }

    fn time_after(&self, last: Option<DateTime<FixedOffset>>) -> DateTime<FixedOffset> {
        let last = last.map(|t| t.with_timezone(&Utc));
        if let Some(c) = &self.clock {
            let t = floor(c());
            return match last {
                Some(l) if t <= l => l + chrono::Duration::seconds(1),
                _ => t,
            }
            .fixed_offset();
        }
        loop {
            let now = Utc::now();
            let t = floor(now);
            match last {
                Some(l) if t <= l => {
                    let wait = (l + chrono::Duration::seconds(1) - now)
                        .to_std()
                        .unwrap_or(Duration::from_millis(10));
                    std::thread::sleep(wait + Duration::from_millis(5));
                }
                _ => return t.fixed_offset(),
            }
        }
    }

    fn signer(&self, name: &str) -> Result<KmSigner, WebvhError> {
        Ok(KmSigner::new(self.keys.clone(), name)?)
    }

    fn multikey(&self, name: &str) -> Result<Multibase, WebvhError> {
        Ok(Multibase::new(self.keys.public_key(name)?.multikey()))
    }

    /// Creates a DID: its genesis entry with `document`, whose `id` must be
    /// [`CreateOptions::template_did`]. The genesis entry still needs its witness proofs when
    /// `options` names witnesses.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Malformed`] when the document's id is not the template DID;
    /// [`WebvhError::Refused`] when the library refuses the entry; key manager errors.
    // @tg: implements:: [[openspec:webvh-identity#Create With Vertep Keys]]
    pub fn create(
        &mut self,
        options: &CreateOptions,
        document: &Value,
    ) -> Result<DidLog, WebvhError> {
        let template = options.template_did();
        if document.get("id").and_then(Value::as_str) != Some(template.as_str()) {
            return Err(WebvhError::Malformed(format!(
                "the document id must be {template}"
            )));
        }
        let signer = self.signer(&self.update.current)?;
        let next_key_hashes = if options.pre_rotation {
            let next = match &self.update.next {
                Some(n) => n.clone(),
                None => {
                    let n = self.keys.generate(KeyType::Ed25519)?;
                    self.update.next = Some(n.clone());
                    n
                }
            };
            let mk = self.keys.public_key(&next)?.multikey();
            Some(Arc::new(vec![Multibase::new(key_hash(&mk))]))
        } else {
            self.update.next = None;
            None
        };
        let params = Parameters {
            update_keys: Some(Arc::new(vec![Multibase::new(signer.multikey())])),
            next_key_hashes,
            witness: options
                .witnesses
                .as_ref()
                .map(Witnessing::parameter)
                .transpose()?,
            ttl: options.ttl,
            portable: Some(false),
            ..Parameters::default()
        };
        let mut state = DIDWebVHState::default();
        let t = self.time_after(None);
        let entry = futures::executor::block_on(state.create_log_entry(
            Some(t),
            document,
            &params,
            &signer,
        ))
        .map_err(|e| WebvhError::Refused(e.to_string()))?;
        let did = entry
            .get_state()
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| WebvhError::Malformed("the created entry has no id".into()))?
            .to_owned();
        let value = serde_json::to_value(&entry.log_entry)
            .map_err(|e| WebvhError::Malformed(e.to_string()))?;
        Ok(DidLog::new(did, vec![value]))
    }

    /// Appends one entry with `change` to `log` and returns its `versionId`. On failure the log
    /// and the update keys are unchanged.
    ///
    /// # Errors
    ///
    /// [`WebvhError::Refused`] for a deactivated DID, a key the log does not authorise, or a
    /// rotation to a key that was not announced; [`WebvhError::Invalid`] when the existing log
    /// does not verify; key manager errors.
    // @tg: implements:: [[openspec:webvh-identity#Update And Rotate]]
    pub fn append(&mut self, log: &mut DidLog, change: EntryChange) -> Result<String, WebvhError> {
        let mut state = log.replay()?;
        let last = state
            .log_entries()
            .last()
            .ok_or_else(|| WebvhError::Malformed("the log is empty".into()))?;
        let prev = last.validated_parameters.clone();
        if prev.deactivated == Some(true) {
            return Err(WebvhError::Refused(format!("{} is deactivated", log.did())));
        }
        let document = change
            .document
            .clone()
            .unwrap_or_else(|| last.get_state().clone());
        let last_time = last.log_entry.get_version_time();

        let mut params = Parameters {
            ttl: change.ttl,
            ..Parameters::default()
        };
        if let Some(w) = &change.witnesses {
            params.witness = Some(match w {
                Some(w) => w.parameter()?,
                None => Arc::new(Witnesses::Empty {}),
            });
        }
        let mut generated = None;
        let (signer_name, new_update) = if prev.pre_rotation_active {
            let to = change
                .rotate_to
                .clone()
                .or_else(|| self.update.next.clone())
                .ok_or_else(|| {
                    WebvhError::Refused(
                        "pre-rotation is active and no announced next key is known".into(),
                    )
                })?;
            params.update_keys = Some(Arc::new(vec![self.multikey(&to)?]));
            let next = if change.end_pre_rotation || change.deactivate {
                params.next_key_hashes = Some(Arc::new(Vec::new()));
                None
            } else {
                let n = self.keys.generate(KeyType::Ed25519)?;
                generated = Some(n.clone());
                let mk = self.keys.public_key(&n)?.multikey();
                params.next_key_hashes = Some(Arc::new(vec![Multibase::new(key_hash(&mk))]));
                Some(n)
            };
            (to.clone(), UpdateKeys { current: to, next })
        } else {
            let current = self.update.current.clone();
            match &change.rotate_to {
                Some(to) => {
                    params.update_keys = Some(Arc::new(vec![self.multikey(to)?]));
                    (
                        current,
                        UpdateKeys {
                            current: to.clone(),
                            next: None,
                        },
                    )
                }
                None => (
                    current.clone(),
                    UpdateKeys {
                        current,
                        next: None,
                    },
                ),
            }
        };
        if change.deactivate {
            if prev.pre_rotation_active {
                return Err(WebvhError::Refused(
                    "end pre-rotation before deactivating (Author::deactivate does both)".into(),
                ));
            }
            params.deactivated = Some(true);
            params.update_keys = Some(Arc::new(Vec::new()));
        }
        let signer = self.signer(&signer_name)?;
        let t = self.time_after(Some(last_time));
        let created = futures::executor::block_on(state.create_log_entry(
            Some(t),
            &document,
            &params,
            &signer,
        ));
        let entry = match created {
            Ok(e) => e,
            Err(e) => {
                if let Some(n) = generated {
                    let _ = self.keys.delete(&n);
                }
                return Err(WebvhError::Refused(e.to_string()));
            }
        };
        let version_id = entry.get_version_id().to_owned();
        let value = serde_json::to_value(&entry.log_entry)
            .map_err(|e| WebvhError::Malformed(e.to_string()))?;
        log.push_entry(value);
        self.update = new_update;
        Ok(version_id)
    }

    /// Replaces the document, keeping the DID.
    ///
    /// # Errors
    ///
    /// As [`Author::append`].
    pub fn update(&mut self, log: &mut DidLog, document: Value) -> Result<String, WebvhError> {
        self.append(
            log,
            EntryChange {
                document: Some(document),
                ..EntryChange::default()
            },
        )
    }

    /// Rotates the update key: to the announced next key with pre-rotation, otherwise to `to`
    /// or a new key.
    ///
    /// # Errors
    ///
    /// As [`Author::append`]; a rotation to a key whose hash was not announced is refused.
    pub fn rotate(&mut self, log: &mut DidLog, to: Option<String>) -> Result<String, WebvhError> {
        let to = match to {
            Some(t) => Some(t),
            None if self.update.next.is_some() => None,
            None => Some(self.keys.generate(KeyType::Ed25519)?),
        };
        self.append(
            log,
            EntryChange {
                rotate_to: to,
                ..EntryChange::default()
            },
        )
    }

    /// Deactivates the DID: with pre-rotation, first an entry that ends it, then the final
    /// entry with no update keys. Returns the final `versionId`.
    ///
    /// # Errors
    ///
    /// As [`Author::append`].
    // @tg: implements:: [[openspec:webvh-identity#Deactivate]]
    pub fn deactivate(&mut self, log: &mut DidLog) -> Result<String, WebvhError> {
        let pre = log
            .replay()?
            .log_entries()
            .last()
            .is_some_and(|e| e.validated_parameters.pre_rotation_active);
        if pre {
            self.append(
                log,
                EntryChange {
                    end_pre_rotation: true,
                    ..EntryChange::default()
                },
            )?;
        }
        self.append(
            log,
            EntryChange {
                deactivate: true,
                ..EntryChange::default()
            },
        )
    }
}
