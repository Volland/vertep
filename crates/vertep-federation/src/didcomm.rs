//! DIDComm Messaging v2 (feature `didcomm`): authcrypt and anoncrypt of relay envelopes, forward
//! wrapping per mediator routing key, and opening, with every private-key operation done by the
//! actor's [`KeyManager`].
//!
//! The JOSE building blocks (Concat KDF, AES-256 key wrap, A256CBC-HS512, ephemeral X25519 keys,
//! the JWE envelope types) are Affinidi's maintained DIDComm v2.1 crates. Their high-level
//! pack/unpack functions take raw private keys, so this module composes the same JWE from those
//! primitives with the static-key ECDH steps (`Zs` for authcrypt, `Ze` and `Zs` on receipt)
//! delegated to [`KeyManager::key_agreement`]. Output is wire-compatible with Affinidi's own
//! `authcrypt`/`anoncrypt` and `decrypt` (checked by the tests).
//!
//! [`DidCommProtocol`] is the relay protocol the runtime integration uses; mediator pickup and
//! the mailbox are protocol-neutral ([`crate::inbox`]).

use std::fmt;
use std::sync::Arc;

use affinidi_crypto::jose::concat_kdf::{concat_kdf, concat_kdf_1pu};
use affinidi_crypto::jose::key_agreement::{Curve, EphemeralKeyPair, PublicKeyAgreement};
use affinidi_crypto::jose::{aes_kw, content_encryption};
use affinidi_messaging_didcomm::jwe::encrypt::anoncrypt as jwe_anoncrypt;
use affinidi_messaging_didcomm::jwe::envelope::{
    ALG_ANONCRYPT, ALG_AUTHCRYPT, Jwe, PerRecipientHeader, ProtectedHeader, Recipient,
};
use affinidi_messaging_didcomm::message::forward::FORWARD_MESSAGE_TYPE;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::did::{Did, DidDocument};
use crate::directory::Route;
use crate::envelope::RelayEnvelope;
use crate::error::{CommunicationError, CommunicationFailure};
use crate::identity::{ActorIdentity, KeyLookupError};
use crate::keys::{KeyManager, PublicKey};
use crate::origin::{OpenedMessage, Refusal, VerifiedOrigin};
use crate::resolver::DidResolver;
use crate::time::Timestamp;
use crate::vocab::{DIDCOMM_EVENT_TYPE, DIDCOMM_PROCESSOR};

const ENC: &str = "A256CBC-HS512";

/// The DIDComm plaintext message that carries a relay envelope.
#[derive(serde::Serialize)]
struct EventMessage<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    type_: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<&'a str>,
    to: [&'a str; 1],
    body: &'a RelayEnvelope,
}

/// The body of a received event message, read with the payload's member order intact.
#[derive(serde::Deserialize)]
struct IncomingEvent {
    body: RelayEnvelope,
}
const TYP: &str = "application/didcomm-encrypted+json";

/// A DIDComm failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DidCommError {
    /// The recipient (or a routing key) has no usable key-agreement key.
    NoKey(String),
    /// Sealing failed.
    Seal(String),
    /// The message is not addressed to any key of this actor.
    NotForMe,
    /// The message did not decrypt (tampered, wrong key, key retired beyond its grace period).
    Undecryptable {
        /// The key id it was addressed to, if one of ours.
        kid: Option<String>,
        /// The reason.
        reason: String,
    },
    /// The sender key does not authenticate the message, or does not belong to the claimed
    /// sender.
    ForgedOrigin {
        /// The claimed sender.
        claimed: String,
        /// The reason.
        reason: String,
    },
    /// The plaintext is not a message this layer understands.
    Malformed(String),
}

impl fmt::Display for DidCommError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DidCommError::NoKey(m) => write!(f, "no DIDComm key: {m}"),
            DidCommError::Seal(m) => write!(f, "DIDComm seal failed: {m}"),
            DidCommError::NotForMe => f.write_str("message not addressed to this actor"),
            DidCommError::Undecryptable { reason, .. } => write!(f, "undecryptable: {reason}"),
            DidCommError::ForgedOrigin { claimed, reason } => {
                write!(f, "sender {claimed} not authenticated: {reason}")
            }
            DidCommError::Malformed(m) => write!(f, "malformed DIDComm message: {m}"),
        }
    }
}

impl std::error::Error for DidCommError {}

impl DidCommError {
    /// The refusal to record for a message that failed to open.
    pub fn to_refusal(&self) -> Refusal {
        match self {
            DidCommError::ForgedOrigin { claimed, reason } => Refusal::ForgedOrigin {
                claimed: claimed.clone(),
                reason: reason.clone(),
            },
            DidCommError::Malformed(m) => Refusal::MalformedEnvelope(m.clone()),
            DidCommError::Undecryptable { kid, reason } => Refusal::Undecryptable {
                reason: reason.clone(),
                kid: kid.clone(),
                sender: None,
            },
            other => Refusal::Undecryptable {
                reason: other.to_string(),
                kid: None,
                sender: None,
            },
        }
    }
}

impl From<DidCommError> for CommunicationError {
    fn from(e: DidCommError) -> Self {
        CommunicationError::new(match e {
            DidCommError::NoKey(_) => CommunicationFailure::NoCompatibleEndpoint,
            other => CommunicationFailure::SealFailed(other.to_string()),
        })
    }
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(s: &str, what: &str) -> Result<Vec<u8>, DidCommError> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|e| DidCommError::Malformed(format!("{what}: {e}")))
}

fn apv_of(kids: &[&str]) -> Vec<u8> {
    let mut sorted: Vec<&str> = kids.to_vec();
    sorted.sort_unstable();
    Sha256::digest(sorted.join(".").as_bytes()).to_vec()
}

fn random_id() -> String {
    let mut b = [0u8; 16];
    // A failing system RNG leaves zeros; the id is a label, not a secret.
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The key-agreement keys of a document as `(kid, X25519 key)`.
pub fn agreement_keys(doc: &DidDocument) -> Vec<(String, PublicKey)> {
    doc.key_agreement_methods()
        .into_iter()
        .filter_map(|m| m.public_key.to_x25519().ok().map(|k| (m.id.clone(), k)))
        .collect()
}

/// Anonymously encrypts `plaintext` to the given recipient keys (ECDH-ES+A256KW, A256CBC-HS512).
///
/// # Errors
///
/// [`DidCommError::Seal`] when encryption fails.
pub fn anoncrypt(
    plaintext: &[u8],
    recipients: &[(String, PublicKey)],
) -> Result<String, DidCommError> {
    if recipients.is_empty() {
        return Err(DidCommError::NoKey("no recipient keys".into()));
    }
    let keys: Vec<(String, PublicKeyAgreement)> = recipients
        .iter()
        .map(|(kid, k)| (kid.clone(), PublicKeyAgreement::X25519(k.bytes)))
        .collect();
    let refs: Vec<(&str, &PublicKeyAgreement)> =
        keys.iter().map(|(k, p)| (k.as_str(), p)).collect();
    jwe_anoncrypt(plaintext, &refs).map_err(|e| DidCommError::Seal(e.to_string()))
}

/// What an opened DIDComm message carried.
#[derive(Clone, Debug, PartialEq)]
pub enum DidCommReceived {
    /// An event with its verified origin.
    Event(Box<OpenedMessage>),
    /// A forward this actor (a mediator) is to pass on: the next hop and the still-encrypted
    /// inner message, which it cannot read.
    Forward {
        /// The next recipient (a DID or a routing key id).
        next: String,
        /// The inner JWE.
        message: String,
    },
}

/// A message ready for its first hop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    /// Where to post it: the service endpoint (a mediator) or the recipient's own endpoint.
    pub endpoint: String,
    /// The outermost JWE.
    pub message: String,
}

/// A decrypted DIDComm plaintext message.
#[derive(Clone, Debug, PartialEq)]
pub struct Plaintext {
    /// The message.
    pub message: Value,
    /// Its bytes as decrypted, with the payload's member order intact.
    pub bytes: Vec<u8>,
    /// The sender DID and the key id that authenticated it, for an authcrypted message; `None`
    /// for an anoncrypted one.
    pub sender: Option<(Did, String)>,
}

/// One actor's DIDComm side: its identity and key manager.
// @tg: implements:: [[openspec:relay-mailboxes#DIDComm Forward Routing]]
pub struct DidCommAgent {
    identity: ActorIdentity,
    km: Arc<dyn KeyManager>,
}

impl fmt::Debug for DidCommAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DidCommAgent")
            .field("did", &self.identity.did)
            .finish()
    }
}

impl DidCommAgent {
    /// The agent of `identity`, whose keys `km` holds.
    pub fn new(identity: ActorIdentity, km: Arc<dyn KeyManager>) -> Self {
        DidCommAgent { identity, km }
    }

    /// The identity.
    pub fn identity(&self) -> &ActorIdentity {
        &self.identity
    }

    fn event_plaintext(&self, to: &str, envelope: &RelayEnvelope, from: bool) -> Vec<u8> {
        // A typed message keeps the payload's member order (a `serde_json::Value` would sort it).
        serde_json::to_vec(&EventMessage {
            id: &envelope.message_id,
            type_: DIDCOMM_EVENT_TYPE,
            from: from.then_some(self.identity.did.as_str()),
            to: [to],
            body: envelope,
        })
        .unwrap_or_default()
    }

    /// Authcrypts `plaintext` from this actor's key-agreement key to `recipients`
    /// (ECDH-1PU+A256KW, A256CBC-HS512). The sender's static ECDH goes through the key manager.
    ///
    /// # Errors
    ///
    /// [`DidCommError`] for missing keys and key manager failures.
    pub fn authcrypt(
        &self,
        plaintext: &[u8],
        recipients: &[(String, PublicKey)],
    ) -> Result<String, DidCommError> {
        if recipients.is_empty() {
            return Err(DidCommError::NoKey("no recipient keys".into()));
        }
        let skid = self.identity.agreement.kid.as_str();
        let ephemeral = EphemeralKeyPair::generate(Curve::X25519);
        let kids: Vec<&str> = recipients.iter().map(|(k, _)| k.as_str()).collect();
        let apv = apv_of(&kids);
        let header = ProtectedHeader {
            typ: Some(TYP.into()),
            alg: ALG_AUTHCRYPT.into(),
            enc: ENC.into(),
            skid: Some(skid.to_owned()),
            apu: Some(b64(skid.as_bytes())),
            apv: b64(&apv),
            epk: ephemeral.public.to_jwk(),
        };
        let protected = b64(serde_json::to_string(&header)
            .map_err(|e| DidCommError::Seal(e.to_string()))?
            .as_bytes());
        let cek = zeroize::Zeroizing::new(content_encryption::generate_cek());
        let iv = content_encryption::generate_iv();
        let (ciphertext, tag) =
            content_encryption::encrypt(plaintext, &cek, &iv, protected.as_bytes())
                .map_err(|e| DidCommError::Seal(e.to_string()))?;
        let mut out = Vec::with_capacity(recipients.len());
        for (kid, key) in recipients {
            let ze = ephemeral
                .private
                .diffie_hellman(&PublicKeyAgreement::X25519(key.bytes))
                .map_err(|e| DidCommError::Seal(e.to_string()))?;
            let zs = self
                .km
                .key_agreement(&self.identity.agreement.key_name, key)
                .map_err(|e| DidCommError::Seal(e.to_string()))?;
            let mut z = zeroize::Zeroizing::new(Vec::with_capacity(64));
            z.extend_from_slice(&ze);
            z.extend_from_slice(zs.as_bytes());
            let kek = zeroize::Zeroizing::new(
                concat_kdf_1pu(
                    &z,
                    ALG_AUTHCRYPT.as_bytes(),
                    skid.as_bytes(),
                    &apv,
                    256,
                    &tag,
                )
                .map_err(|e| DidCommError::Seal(e.to_string()))?,
            );
            let kek: &[u8; 32] = kek
                .as_slice()
                .try_into()
                .map_err(|_| DidCommError::Seal("KEK size".into()))?;
            let wrapped =
                aes_kw::wrap(kek, cek.as_slice()).map_err(|e| DidCommError::Seal(e.to_string()))?;
            out.push(Recipient {
                header: PerRecipientHeader { kid: kid.clone() },
                encrypted_key: b64(&wrapped),
            });
        }
        serde_json::to_string(&Jwe {
            protected,
            recipients: out,
            iv: b64(&iv),
            ciphertext: b64(&ciphertext),
            tag: b64(&tag),
        })
        .map_err(|e| DidCommError::Seal(e.to_string()))
    }

    /// Encrypts `envelope` to the recipient `to` (authcrypt, or anoncrypt when `anonymous`), then
    /// wraps it in one forward message per routing key of `route`, each anoncrypted to its
    /// mediator, so each mediator can open only its own layer. `resolver` resolves the routing
    /// keys' DIDs.
    ///
    /// # Errors
    ///
    /// [`DidCommError`] when a key is missing or sealing fails.
    pub fn seal_event(
        &self,
        to: &DidDocument,
        route: &Route,
        envelope: &RelayEnvelope,
        anonymous: bool,
        resolver: &dyn DidResolver,
    ) -> Result<Outgoing, DidCommError> {
        let keys = agreement_keys(to);
        if keys.is_empty() {
            return Err(DidCommError::NoKey(format!(
                "{}: no keyAgreement key",
                to.id
            )));
        }
        let plaintext = self.event_plaintext(&to.id, envelope, !anonymous);
        let inner = if anonymous {
            anoncrypt(&plaintext, &keys)?
        } else {
            self.authcrypt(&plaintext, &keys)?
        };
        let message = wrap_forwards(&to.id, inner, &route.endpoint.routing_keys, resolver)?;
        Ok(Outgoing {
            endpoint: route.endpoint.uri.clone(),
            message,
        })
    }

    /// Authcrypts the plaintext DIDComm message `message` to every key-agreement key of `to`.
    /// Protocol messages (mediator coordination, pickup) go through here.
    ///
    /// # Errors
    ///
    /// [`DidCommError`] when `to` has no key-agreement key or sealing fails.
    pub fn seal_message(&self, message: &Value, to: &DidDocument) -> Result<String, DidCommError> {
        let keys = agreement_keys(to);
        if keys.is_empty() {
            return Err(DidCommError::NoKey(format!(
                "{}: no keyAgreement key",
                to.id
            )));
        }
        let bytes = serde_json::to_vec(message).map_err(|e| DidCommError::Seal(e.to_string()))?;
        self.authcrypt(&bytes, &keys)
    }

    /// Opens a JWE addressed to one of this actor's key-agreement keys (the current one, or a
    /// retired one within its grace period at `now`). Authcrypted senders are authenticated
    /// against their DID document from `resolver`; anoncrypted messages have no verified origin.
    ///
    /// # Errors
    ///
    /// [`DidCommError`] naming why the message cannot be delivered.
    // @tg: implements:: [[openspec:did-identity#Authenticated Event Origin]]
    pub fn open(
        &self,
        jwe: &str,
        resolver: &dyn DidResolver,
        now: Timestamp,
    ) -> Result<DidCommReceived, DidCommError> {
        let Plaintext {
            message: msg,
            bytes: plaintext,
            sender,
        } = self.decrypt(jwe, resolver, now)?;
        match msg.get("type").and_then(Value::as_str) {
            Some(FORWARD_MESSAGE_TYPE) => {
                let next = msg
                    .pointer("/body/next")
                    .and_then(Value::as_str)
                    .ok_or_else(|| DidCommError::Malformed("forward without next".into()))?
                    .to_owned();
                let inner = msg
                    .pointer("/attachments/0/data/json")
                    .ok_or_else(|| DidCommError::Malformed("forward without attachment".into()))?;
                Ok(DidCommReceived::Forward {
                    next,
                    message: inner.to_string(),
                })
            }
            Some(DIDCOMM_EVENT_TYPE) => {
                let envelope = serde_json::from_slice::<IncomingEvent>(&plaintext)
                    .map_err(|e| DidCommError::Malformed(e.to_string()))?
                    .body;
                RelayEnvelope::from_bytes(&envelope.to_bytes())
                    .map_err(|e| DidCommError::Malformed(e.to_string()))?;
                let origin = match sender {
                    Some((did, skid)) => {
                        // DIDComm requires `from` to name the DID of the sender key.
                        if msg.get("from").and_then(Value::as_str) != Some(did.as_str()) {
                            return Err(DidCommError::ForgedOrigin {
                                claimed: msg
                                    .get("from")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned(),
                                reason: format!("from does not match sender key {skid}"),
                            });
                        }
                        VerifiedOrigin::verified(did.as_str(), skid, DIDCOMM_PROCESSOR)
                    }
                    // An anonymous message's `from`, if any, is a claim nobody verified.
                    None => VerifiedOrigin::anonymous(DIDCOMM_PROCESSOR),
                };
                Ok(DidCommReceived::Event(Box::new(OpenedMessage {
                    envelope,
                    origin,
                })))
            }
            other => Err(DidCommError::Malformed(format!(
                "unsupported message type {other:?}"
            ))),
        }
    }

    /// Decrypts a JWE addressed to one of this actor's key-agreement keys and authenticates an
    /// authcrypted sender against its resolved DID document. Returns the plaintext message,
    /// whatever its type.
    ///
    /// # Errors
    ///
    /// [`DidCommError`] naming why the message cannot be opened.
    pub fn decrypt(
        &self,
        jwe: &str,
        resolver: &dyn DidResolver,
        now: Timestamp,
    ) -> Result<Plaintext, DidCommError> {
        let jwe: Jwe =
            serde_json::from_str(jwe).map_err(|e| DidCommError::Malformed(e.to_string()))?;
        let header = ProtectedHeader::from_base64url(&jwe.protected)
            .map_err(|e| DidCommError::Malformed(e.to_string()))?;
        // Find the recipient entry addressed to one of our keys.
        let mut chosen = None;
        let mut retired_err = None;
        for r in &jwe.recipients {
            match self.identity.decryption_key(&r.header.kid, now) {
                Ok(k) => {
                    chosen = Some((r, k.key_name.clone()));
                    break;
                }
                Err(e @ KeyLookupError::RetiredBeyondGrace { .. }) => {
                    retired_err = Some((r.header.kid.clone(), e));
                }
                Err(KeyLookupError::Unknown(_)) => {}
            }
        }
        let (recipient, key_name) = match (chosen, retired_err) {
            (Some(c), _) => c,
            (None, Some((kid, e))) => {
                return Err(DidCommError::Undecryptable {
                    kid: Some(kid),
                    reason: e.to_string(),
                });
            }
            (None, None) => return Err(DidCommError::NotForMe),
        };
        let kid = recipient.header.kid.clone();
        let undecryptable = |reason: String| DidCommError::Undecryptable {
            kid: Some(kid.clone()),
            reason,
        };
        let epk = match PublicKeyAgreement::from_jwk(&header.epk) {
            Ok(PublicKeyAgreement::X25519(b)) => PublicKey::x25519(b),
            _ => return Err(DidCommError::Malformed("epk is not an X25519 key".into())),
        };
        let apv = unb64(&header.apv, "apv")?;
        let iv: [u8; 16] = unb64(&jwe.iv, "iv")?
            .try_into()
            .map_err(|_| DidCommError::Malformed("iv size".into()))?;
        let tag: [u8; 32] = unb64(&jwe.tag, "tag")?
            .try_into()
            .map_err(|_| DidCommError::Malformed("tag size".into()))?;
        let ciphertext = unb64(&jwe.ciphertext, "ciphertext")?;
        let wrapped = unb64(&recipient.encrypted_key, "encrypted_key")?;
        let ze = self
            .km
            .key_agreement(&key_name, &epk)
            .map_err(|e| undecryptable(e.to_string()))?;

        let (kek, sender) = match header.alg.as_str() {
            ALG_AUTHCRYPT => {
                let skid = header
                    .authcrypt_sender_kid()
                    .map_err(|e| DidCommError::Malformed(e.to_string()))?
                    .ok_or_else(|| DidCommError::Malformed("authcrypt without skid".into()))?
                    .to_owned();
                let sender_did =
                    Did::from_did_url(&skid).map_err(|e| DidCommError::Malformed(e.to_string()))?;
                let forged = |reason: String| DidCommError::ForgedOrigin {
                    claimed: sender_did.to_string(),
                    reason,
                };
                let doc = resolver
                    .resolve(&sender_did)
                    .map_err(|e| forged(e.to_string()))?
                    .document;
                let sender_key = agreement_keys(&doc)
                    .into_iter()
                    .find(|(k, _)| *k == skid)
                    .ok_or_else(|| {
                        forged(format!("{skid} is not a key-agreement key of the sender"))
                    })?
                    .1;
                let zs = self
                    .km
                    .key_agreement(&key_name, &sender_key)
                    .map_err(|e| undecryptable(e.to_string()))?;
                let mut z = zeroize::Zeroizing::new(Vec::with_capacity(64));
                z.extend_from_slice(ze.as_bytes());
                z.extend_from_slice(zs.as_bytes());
                let kek = concat_kdf_1pu(
                    &z,
                    ALG_AUTHCRYPT.as_bytes(),
                    skid.as_bytes(),
                    &apv,
                    256,
                    &tag,
                )
                .map_err(|e| undecryptable(e.to_string()))?;
                (kek, Some((sender_did, skid)))
            }
            ALG_ANONCRYPT => {
                let apu = match &header.apu {
                    Some(a) => unb64(a, "apu")?,
                    None => Vec::new(),
                };
                let kek = concat_kdf(ze.as_bytes(), ALG_ANONCRYPT.as_bytes(), &apu, &apv, 256)
                    .map_err(|e| undecryptable(e.to_string()))?;
                (kek, None)
            }
            other => return Err(DidCommError::Malformed(format!("unsupported alg {other}"))),
        };
        let kek = zeroize::Zeroizing::new(kek);
        let kek: &[u8; 32] = kek
            .as_slice()
            .try_into()
            .map_err(|_| undecryptable("KEK size".into()))?;
        let unwrapped = aes_kw::unwrap(kek, &wrapped).map_err(|e| match &sender {
            // A wrong sender key yields a wrong KEK: the claimed sender is not authenticated.
            Some((did, _)) => DidCommError::ForgedOrigin {
                claimed: did.to_string(),
                reason: e.to_string(),
            },
            None => undecryptable(e.to_string()),
        })?;
        let cek = zeroize::Zeroizing::new(unwrapped);
        let cek: &[u8; 64] = cek
            .as_slice()
            .try_into()
            .map_err(|_| undecryptable("CEK size".into()))?;
        let plaintext =
            content_encryption::decrypt(&ciphertext, cek, &iv, jwe.protected.as_bytes(), &tag)
                .map_err(|e| undecryptable(e.to_string()))?;
        let message: Value = serde_json::from_slice(&plaintext)
            .map_err(|e| DidCommError::Malformed(e.to_string()))?;
        Ok(Plaintext {
            message,
            bytes: plaintext,
            sender,
        })
    }
}

/// Wraps `inner` (already encrypted to `recipient`) in one forward per routing key, innermost
/// for the last key, each anoncrypted to that key. `routing_keys` are DID URLs resolved with
/// `resolver`.
///
/// # Errors
///
/// [`DidCommError`] when a routing key cannot be resolved or sealing fails.
pub fn wrap_forwards(
    recipient: &str,
    inner: String,
    routing_keys: &[String],
    resolver: &dyn DidResolver,
) -> Result<String, DidCommError> {
    let mut message = inner;
    let mut next = recipient.to_owned();
    for rk in routing_keys.iter().rev() {
        let did = Did::from_did_url(rk).map_err(|e| DidCommError::NoKey(e.to_string()))?;
        let doc = resolver
            .resolve(&did)
            .map_err(|e| DidCommError::NoKey(format!("{rk}: {e}")))?
            .document;
        let key = agreement_keys(&doc)
            .into_iter()
            .find(|(k, _)| k == rk)
            .ok_or_else(|| DidCommError::NoKey(format!("{rk} is not a key-agreement key")))?;
        let inner_json: Value =
            serde_json::from_str(&message).map_err(|e| DidCommError::Seal(e.to_string()))?;
        let forward = json!({
            "id": random_id(),
            "type": FORWARD_MESSAGE_TYPE,
            "to": [did.as_str()],
            "body": {"next": next},
            "attachments": [{"data": {"json": inner_json}}],
        });
        message = anoncrypt(
            &serde_json::to_vec(&forward).map_err(|e| DidCommError::Seal(e.to_string()))?,
            &[key],
        )?;
        next = rk.clone();
    }
    Ok(message)
}

/// Anonymously encrypts `envelope` to the recipient `to` (no `from`, so no sender to verify)
/// and wraps it in one forward per routing key of `route`.
///
/// # Errors
///
/// [`DidCommError`] when a key is missing or sealing fails.
pub fn seal_anonymous_event(
    to: &DidDocument,
    route: &Route,
    envelope: &RelayEnvelope,
    resolver: &dyn DidResolver,
) -> Result<Outgoing, DidCommError> {
    let keys = agreement_keys(to);
    if keys.is_empty() {
        return Err(DidCommError::NoKey(format!(
            "{}: no keyAgreement key",
            to.id
        )));
    }
    let plaintext = serde_json::to_vec(&EventMessage {
        id: &envelope.message_id,
        type_: DIDCOMM_EVENT_TYPE,
        from: None,
        to: [to.id.as_str()],
        body: envelope,
    })
    .map_err(|e| DidCommError::Seal(e.to_string()))?;
    let inner = anoncrypt(&plaintext, &keys)?;
    let message = wrap_forwards(&to.id, inner, &route.endpoint.routing_keys, resolver)?;
    Ok(Outgoing {
        endpoint: route.endpoint.uri.clone(),
        message,
    })
}

/// The DIDs a JWE is addressed to, from its recipients' key ids (`did#key`): routing data
/// every relay can read.
pub fn jwe_recipients(jwe: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(jwe) else {
        return Vec::new();
    };
    let mut out: Vec<String> = v
        .get("recipients")
        .and_then(Value::as_array)
        .map(|rs| {
            rs.iter()
                .filter_map(|r| r.pointer("/header/kid").and_then(Value::as_str))
                .map(|kid| kid.split('#').next().unwrap_or(kid).to_owned())
                .collect()
        })
        .unwrap_or_default();
    out.dedup();
    out
}

/// DIDComm v2 as a relay protocol: authcrypt from a sender with a DID (anoncrypt otherwise),
/// forward wrapping per routing key, a post to the service endpoint; opening through the
/// recipient's key manager with the sender authenticated against its DID document.
#[derive(Clone, Copy, Debug, Default)]
// @tg: implements:: [[openspec:relay-mailboxes#DIDComm Forward Routing]]
// @tg: implements:: [[openspec:relay-mailboxes#End-To-End Encryption Through Relays]]
pub struct DidCommProtocol;

impl crate::relay::RelayProtocol for DidCommProtocol {
    fn kind(&self) -> crate::directory::TransportKind {
        crate::directory::TransportKind::DidComm
    }

    fn send(&self, cx: &crate::relay::SendContext<'_>) -> Result<(), crate::relay::SendError> {
        use crate::relay::SendError;
        let sealed = match cx.from {
            Some(from) => DidCommAgent::new(from.identity.clone(), from.km.clone()).seal_event(
                &cx.to.document,
                cx.route,
                cx.envelope,
                false,
                cx.resolver,
            ),
            None => seal_anonymous_event(&cx.to.document, cx.route, cx.envelope, cx.resolver),
        };
        let out = sealed.map_err(|e| match e {
            // A routing key that no longer resolves to a key: the document may be stale.
            DidCommError::NoKey(m) => SendError::Stale(m),
            other => SendError::Failed(CommunicationError::from(other).failure),
        })?;
        cx.net.post(&out.endpoint, out.message.as_bytes())?;
        Ok(())
    }

    fn open(
        &self,
        to: &crate::relay::LocalActor,
        bytes: &[u8],
        resolver: &dyn DidResolver,
        now: Timestamp,
    ) -> Result<OpenedMessage, Refusal> {
        let jwe =
            std::str::from_utf8(bytes).map_err(|e| Refusal::MalformedEnvelope(e.to_string()))?;
        match DidCommAgent::new(to.identity.clone(), to.km.clone()).open(jwe, resolver, now) {
            Ok(DidCommReceived::Event(m)) => Ok(*m),
            Ok(DidCommReceived::Forward { .. }) => Err(Refusal::MalformedEnvelope(
                "a forward reached an actor, not a mediator".into(),
            )),
            Err(e) => Err(e.to_refusal()),
        }
    }
}

/// A node's own DIDComm endpoint for routes without a mediator: it stores each posted message
/// for the DID its recipient keys name, until the recipient's mailbox picks it up.
#[derive(Debug)]
pub struct DidCommInbound(pub Arc<crate::net::DropBox>);

impl crate::net::RelayEndpoint for DidCommInbound {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, crate::net::NetError> {
        let jwe =
            std::str::from_utf8(body).map_err(|e| crate::net::NetError::Refused(e.to_string()))?;
        let to = jwe_recipients(jwe);
        let Some(did) = to.first() else {
            return Err(crate::net::NetError::Refused("no recipient".into()));
        };
        self.0.store(did, body.to_vec());
        Ok(Vec::new())
    }
}
