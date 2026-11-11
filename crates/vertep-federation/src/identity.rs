//! Actor identities: the DID an actor uses as its address, and the names of the keys behind it.
//!
//! An [`ActorIdentity`] holds only DIDs, verification method ids and key names: it can be stored
//! with the actor's configuration, journaled or put in the graph without exposing a private key.
//! Every private operation goes through the [`KeyManager`] that holds the named keys.
//!
//! Identities are opt-in per actor type ([`IdentityConfig`]): a virtual actor type configured with
//! a DID method gets a fresh DID for each instance, which becomes the instance's address.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::did::{DidDocument, Service, VerificationMethod};
use crate::keys::{KeyError, KeyManager, KeyType, PublicKey};
use crate::resolver::key::{did_key, key_document};
use crate::resolver::peer::{PeerPurpose, did_peer_2};
use crate::time::Timestamp;

/// How the instances of an actor type get their DID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DidMethodChoice {
    /// `did:key` from one Ed25519 key (key agreement through the derived X25519 key). The DID is
    /// the key, so it cannot rotate.
    Key,
    /// `did:peer:2` with an Ed25519 authentication key, an X25519 key-agreement key and the
    /// given service entries (ids as `#fragment`). Pairwise; the DID binds the keys.
    Peer {
        /// The services (mailboxes and transports) to publish.
        services: Vec<Service>,
    },
    /// A DID whose document the application publishes (`did:web`, `did:webvh`): keys are created
    /// here and [`ActorIdentity::document`] gives the document to publish. Rotates without
    /// changing the DID.
    Hosted {
        /// The DID.
        did: String,
        /// The services to publish.
        services: Vec<Service>,
    },
}

/// The DID configuration of one actor type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityConfig {
    /// The DID method instances get.
    pub method: DidMethodChoice,
    /// How long messages encrypted to a retired key-agreement key stay decryptable.
    pub rotation_grace: Duration,
}

impl IdentityConfig {
    /// A configuration with `method` and a one-day grace period.
    pub fn new(method: DidMethodChoice) -> Self {
        IdentityConfig {
            method,
            rotation_grace: Duration::from_secs(86_400),
        }
    }
}

/// A key of an identity: its verification method id and its name in the key manager.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRef {
    /// The verification method id (a DID URL).
    pub kid: String,
    /// The key's name in the key manager.
    pub key_name: String,
}

/// A superseded key, kept for decryption during the grace period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetiredKey {
    /// The key.
    pub key: KeyRef,
    /// When it was superseded (milliseconds since the epoch).
    pub retired_at: i64,
}

/// The identity of a DID actor: its DID (the actor's address) and the names of its keys.
// @tg: implements:: [[openspec:did-identity#Opt-In DID Identity Per Actor]] {note: "per-actor-type DID provisioning; the runtime binding is Federation::provision"}
// @tg: implements:: [[openspec:did-identity#Key Rotation Without Address Change]]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorIdentity {
    /// The DID, which is the actor's address IRI.
    pub did: String,
    /// The authentication and assertion key (Ed25519).
    pub signing: KeyRef,
    /// The key-agreement key.
    pub agreement: KeyRef,
    /// Superseded key-agreement keys, newest last.
    pub retired: Vec<RetiredKey>,
    /// The service entries published in the document.
    pub services: Vec<Service>,
    /// Whether keys can rotate without changing the DID.
    pub rotatable: bool,
    /// The grace period for retired keys, in milliseconds.
    pub grace_ms: u64,
}

/// Why a message's key-agreement key cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyLookupError {
    /// The key id is not one of this identity's keys.
    Unknown(String),
    /// The key was retired before the grace period that would still accept it.
    RetiredBeyondGrace {
        /// The key id.
        kid: String,
        /// When it was retired.
        retired_at: Timestamp,
    },
}

impl std::fmt::Display for KeyLookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyLookupError::Unknown(k) => write!(f, "{k} is not a key of this actor"),
            KeyLookupError::RetiredBeyondGrace { kid, retired_at } => {
                write!(
                    f,
                    "{kid} was retired at {retired_at}, past the grace period"
                )
            }
        }
    }
}

impl std::error::Error for KeyLookupError {}

impl ActorIdentity {
    /// Creates the keys and the DID of a new actor instance, per `config`.
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the key manager cannot create the keys.
    pub fn provision(km: &dyn KeyManager, config: &IdentityConfig) -> Result<Self, KeyError> {
        let grace_ms = u64::try_from(config.rotation_grace.as_millis()).unwrap_or(u64::MAX);
        match &config.method {
            DidMethodChoice::Key => {
                let name = km.generate(KeyType::Ed25519)?;
                let pk = km.public_key(&name)?;
                let did = did_key(&pk);
                let x = pk.to_x25519()?;
                Ok(ActorIdentity {
                    signing: KeyRef {
                        kid: format!("{did}#{}", pk.multikey()),
                        key_name: name.clone(),
                    },
                    agreement: KeyRef {
                        kid: format!("{did}#{}", x.multikey()),
                        key_name: name,
                    },
                    did,
                    retired: Vec::new(),
                    services: Vec::new(),
                    rotatable: false,
                    grace_ms,
                })
            }
            DidMethodChoice::Peer { services } => {
                let sig = km.generate(KeyType::Ed25519)?;
                let enc = km.generate(KeyType::X25519)?;
                let did = did_peer_2(
                    &[
                        (PeerPurpose::Verification, km.public_key(&sig)?),
                        (PeerPurpose::Encryption, km.public_key(&enc)?),
                    ],
                    services,
                );
                let services = services
                    .iter()
                    .map(|s| Service {
                        id: if s.id.starts_with('#') {
                            format!("{did}{}", s.id)
                        } else {
                            s.id.clone()
                        },
                        ..s.clone()
                    })
                    .collect();
                Ok(ActorIdentity {
                    signing: KeyRef {
                        kid: format!("{did}#key-1"),
                        key_name: sig,
                    },
                    agreement: KeyRef {
                        kid: format!("{did}#key-2"),
                        key_name: enc,
                    },
                    did,
                    retired: Vec::new(),
                    services,
                    rotatable: false,
                    grace_ms,
                })
            }
            DidMethodChoice::Hosted { did, services } => {
                let sig = km.generate(KeyType::Ed25519)?;
                let enc = km.generate(KeyType::X25519)?;
                Ok(ActorIdentity {
                    signing: KeyRef {
                        kid: format!("{did}#{sig}"),
                        key_name: sig,
                    },
                    agreement: KeyRef {
                        kid: format!("{did}#{enc}"),
                        key_name: enc,
                    },
                    did: did.clone(),
                    retired: Vec::new(),
                    services: services
                        .iter()
                        .map(|s| Service {
                            id: if s.id.starts_with('#') {
                                format!("{did}{}", s.id)
                            } else {
                                s.id.clone()
                            },
                            ..s.clone()
                        })
                        .collect(),
                    rotatable: true,
                    grace_ms,
                })
            }
        }
    }

    /// The DID document of this identity: its current public keys and services. For hosted
    /// methods this is what the application publishes after provisioning or rotation.
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the key manager no longer holds the current keys.
    pub fn document(&self, km: &dyn KeyManager) -> Result<DidDocument, KeyError> {
        let sig = km.public_key(&self.signing.key_name)?;
        if self.did.starts_with("did:key:") {
            return key_document(&self.did, &sig.multikey())
                .map_err(|e| KeyError::InvalidKey(e.to_string()));
        }
        let enc: PublicKey = km.public_key(&self.agreement.key_name)?;
        let mut doc = DidDocument::new(&self.did);
        doc.verification_method.push(VerificationMethod::multikey(
            &self.signing.kid,
            &self.did,
            sig,
        ));
        doc.verification_method.push(VerificationMethod::multikey(
            &self.agreement.kid,
            &self.did,
            enc,
        ));
        doc.authentication.push(self.signing.kid.clone());
        doc.assertion_method.push(self.signing.kid.clone());
        doc.key_agreement.push(self.agreement.kid.clone());
        doc.service = self.services.clone();
        Ok(doc)
    }

    /// Replaces both keys with new ones, keeping the DID. The old key-agreement key stays
    /// usable for decryption until the grace period after `at` has passed; the old signing key is
    /// deleted at once. Returns the new document to publish.
    ///
    /// # Errors
    ///
    /// [`KeyError::WrongKeyType`] for methods whose DID is bound to its keys (`did:key`,
    /// `did:peer`), and key manager failures.
    pub fn rotate(&mut self, km: &dyn KeyManager, at: Timestamp) -> Result<DidDocument, KeyError> {
        if !self.rotatable {
            return Err(KeyError::WrongKeyType(format!(
                "{}: the DID is bound to its keys and cannot rotate",
                self.did
            )));
        }
        let sig = km.generate(KeyType::Ed25519)?;
        let enc = km.generate(KeyType::X25519)?;
        let old_sig = std::mem::replace(
            &mut self.signing,
            KeyRef {
                kid: format!("{}#{sig}", self.did),
                key_name: sig,
            },
        );
        let old_enc = std::mem::replace(
            &mut self.agreement,
            KeyRef {
                kid: format!("{}#{enc}", self.did),
                key_name: enc,
            },
        );
        km.delete(&old_sig.key_name)?;
        self.retired.push(RetiredKey {
            key: old_enc,
            retired_at: at.millis(),
        });
        self.document(km)
    }

    /// The key-agreement key to decrypt a message addressed to `kid` at `now`: the current key,
    /// or a retired key still within its grace period.
    ///
    /// # Errors
    ///
    /// [`KeyLookupError`] for unknown keys and keys retired beyond the grace period.
    pub fn decryption_key(&self, kid: &str, now: Timestamp) -> Result<&KeyRef, KeyLookupError> {
        if self.agreement.kid == kid {
            return Ok(&self.agreement);
        }
        let grace = Duration::from_millis(self.grace_ms);
        match self.retired.iter().rev().find(|r| r.key.kid == kid) {
            Some(r) if now < Timestamp(r.retired_at).plus(grace) => Ok(&r.key),
            Some(r) => Err(KeyLookupError::RetiredBeyondGrace {
                kid: kid.to_owned(),
                retired_at: Timestamp(r.retired_at),
            }),
            None => Err(KeyLookupError::Unknown(kid.to_owned())),
        }
    }

    /// Deletes the retired keys whose grace period has passed at `now`. They are kept in
    /// [`ActorIdentity::retired`] so late messages are recognised and recorded.
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the key manager cannot delete a key.
    pub fn purge_retired(&self, km: &dyn KeyManager, now: Timestamp) -> Result<(), KeyError> {
        let grace = Duration::from_millis(self.grace_ms);
        for r in &self.retired {
            if now >= Timestamp(r.retired_at).plus(grace) && km.contains(&r.key.key_name) {
                km.delete(&r.key.key_name)?;
            }
        }
        Ok(())
    }
}
