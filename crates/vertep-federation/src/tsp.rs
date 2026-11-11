//! TSP direct mode (feature `tsp`): sealing and opening Trust Spanning Protocol messages between
//! two actors' VIDs, and setting up the relationship before the first event.
//!
//! The cryptography is the OpenWallet Foundation reference implementation (`tsp_sdk`, CESR
//! encoding, Ed25519 signatures, HPKE/sealed-box encryption). Its `PrivateVid` performs private
//! operations through a `SecureArea`; here that secure area is the actor's [`KeyManager`], so no
//! private key leaves the key manager. An actor's DID is its VID: the peer's VID comes from its
//! resolved DID document (authentication key, key-agreement key and `TSPTransport` endpoint).
//!
//! Routed mode nests the event sealed to the recipient inside routed messages through the
//! intermediaries ([`TspSession::seal_routed`], [`forward_routed`]). [`TspProtocol`] is the relay
//! protocol the runtime integration uses, and [`TspInbound`] a node's TSP endpoint.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

pub use tsp_sdk::Vid;
use tsp_sdk::cesr::EnvelopeType;
use tsp_sdk::definitions::{
    Digest, Payload, PublicKeyData, PublicVerificationKeyData, RelationshipForm,
    VidEncryptionKeyType, VidSignatureKeyType,
};
use tsp_sdk::secure_area::{KeyInfo, KeyType as AreaKeyType, Secret, SecureArea, SecureAreaError};
use tsp_sdk::{PrivateVid, VerifiedVid};
use url::Url;

use crate::did::{Did, DidDocument};
use crate::directory::TransportKind;
use crate::envelope::RelayEnvelope;
use crate::error::{CommunicationError, CommunicationFailure};
use crate::identity::ActorIdentity;
use crate::keys::{KeyError, KeyManager, KeyType, PublicKey};
use crate::net::{DropBox, NetError, RelayEndpoint};
use crate::origin::Refusal;
use crate::origin::{OpenedMessage, VerifiedOrigin};
use crate::relay::{LocalActor, RelayProtocol, SendContext, SendError};
use crate::resolver::DidResolver;
use crate::time::Timestamp;
use crate::vocab::TSP_PROCESSOR;

/// A TSP failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TspError {
    /// The DID document lacks a key or endpoint TSP needs.
    NoVid(String),
    /// Sealing failed.
    Seal(String),
    /// The message did not open: tampered, not addressed to this actor, or not signed by the
    /// sender it names.
    Open(String),
    /// The recipient requires a relationship and none is established.
    RelationshipRequired(String),
    /// The recipient refused the relationship.
    RelationshipRefused(String),
    /// The opened message is not what the exchange expected.
    Unexpected(String),
}

impl fmt::Display for TspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TspError::NoVid(m) => write!(f, "no TSP VID: {m}"),
            TspError::Seal(m) => write!(f, "TSP seal failed: {m}"),
            TspError::Open(m) => write!(f, "TSP open failed: {m}"),
            TspError::RelationshipRequired(m) => write!(f, "TSP relationship required with {m}"),
            TspError::RelationshipRefused(m) => write!(f, "TSP relationship refused by {m}"),
            TspError::Unexpected(m) => write!(f, "unexpected TSP message: {m}"),
        }
    }
}

impl std::error::Error for TspError {}

impl From<TspError> for CommunicationError {
    fn from(e: TspError) -> Self {
        CommunicationError::new(match e {
            TspError::RelationshipRefused(_) | TspError::RelationshipRequired(_) => {
                CommunicationFailure::Refused(e.to_string())
            }
            TspError::NoVid(_) => CommunicationFailure::NoCompatibleEndpoint,
            other => CommunicationFailure::SealFailed(other.to_string()),
        })
    }
}

fn area_error(e: KeyError) -> SecureAreaError {
    match e {
        KeyError::UnknownKey(n) => SecureAreaError::UnknownKey(n),
        KeyError::WrongKeyType(n) => SecureAreaError::WrongKeyType(n),
        KeyError::InvalidKey(m) => SecureAreaError::Malformed(m),
        other => SecureAreaError::Crypto(other.to_string()),
    }
}

/// A [`KeyManager`] seen as a TSP secure area.
#[derive(Clone)]
pub struct KeyManagerArea(pub Arc<dyn KeyManager>);

impl fmt::Debug for KeyManagerArea {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyManagerArea")
    }
}

impl SecureArea for KeyManagerArea {
    fn create_key(&self, key_type: AreaKeyType) -> Result<KeyInfo, SecureAreaError> {
        let kt = match key_type {
            AreaKeyType::Ed25519 => KeyType::Ed25519,
            AreaKeyType::X25519 => KeyType::X25519,
            other => return Err(SecureAreaError::WrongKeyType(other.as_str().to_owned())),
        };
        let name = self.0.generate(kt).map_err(area_error)?;
        let public = self.0.public_key(&name).map_err(area_error)?;
        Ok(KeyInfo {
            name,
            public: public.bytes.to_vec(),
        })
    }

    fn delete_key(&self, name: &str) -> Result<(), SecureAreaError> {
        self.0.delete(name).map_err(area_error)
    }

    fn public_key(&self, name: &str) -> Result<Vec<u8>, SecureAreaError> {
        Ok(self.0.public_key(name).map_err(area_error)?.bytes.to_vec())
    }

    fn key_type(&self, name: &str) -> Result<AreaKeyType, SecureAreaError> {
        Ok(
            match self.0.public_key(name).map_err(area_error)?.key_type {
                KeyType::Ed25519 => AreaKeyType::Ed25519,
                KeyType::X25519 => AreaKeyType::X25519,
            },
        )
    }

    fn sign(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, SecureAreaError> {
        self.0.sign(name, data).map_err(area_error)
    }

    fn key_agreement(&self, name: &str, other_public: &[u8]) -> Result<Secret, SecureAreaError> {
        let peer: [u8; 32] = other_public
            .try_into()
            .map_err(|_| SecureAreaError::Malformed("peer key is not 32 bytes".into()))?;
        let s = self
            .0
            .key_agreement(name, &PublicKey::x25519(peer))
            .map_err(area_error)?;
        Ok(Secret::new(s.as_bytes().to_vec()))
    }

    fn kem_decapsulate(&self, name: &str, _encapsulated: &[u8]) -> Result<Secret, SecureAreaError> {
        Err(SecureAreaError::WrongKeyType(name.to_owned()))
    }
}

/// The first `TSPTransport` endpoint of a DID document.
fn tsp_endpoint(doc: &DidDocument) -> Result<Url, TspError> {
    doc.service
        .iter()
        .filter(|s| s.has_type("TSPTransport") || s.has_type("TspTransport"))
        .flat_map(|s| s.endpoints.iter())
        .find_map(|e| Url::parse(&e.uri).ok())
        .ok_or_else(|| TspError::NoVid(format!("{}: no TSPTransport endpoint", doc.id)))
}

/// The VID of a DID actor from its resolved document: its first authentication key (Ed25519),
/// its first key-agreement key (X25519) and its `TSPTransport` endpoint.
///
/// # Errors
///
/// [`TspError::NoVid`] when a key or the endpoint is missing.
pub fn vid_from_document(doc: &DidDocument) -> Result<Vid, TspError> {
    let sig = doc
        .authentication_methods()
        .into_iter()
        .find(|m| m.public_key.key_type == KeyType::Ed25519)
        .ok_or_else(|| TspError::NoVid(format!("{}: no Ed25519 authentication key", doc.id)))?
        .public_key;
    let enc = doc
        .key_agreement_methods()
        .into_iter()
        .find_map(|m| m.public_key.to_x25519().ok())
        .ok_or_else(|| TspError::NoVid(format!("{}: no key-agreement key", doc.id)))?;
    Ok(Vid::new(
        doc.id.clone(),
        tsp_endpoint(doc)?,
        VidSignatureKeyType::Ed25519,
        PublicVerificationKeyData::from(sig.bytes.to_vec()),
        VidEncryptionKeyType::X25519,
        PublicKeyData::from(enc.bytes.to_vec()),
    ))
}

/// An actor's own VID: its public half and its keys in the key manager.
#[derive(Clone)]
pub struct TspIdentity {
    vid: Vid,
    area: KeyManagerArea,
    signing_name: String,
    agreement_name: String,
}

impl fmt::Debug for TspIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspIdentity")
            .field("vid", &self.vid.identifier())
            .finish()
    }
}

impl TspIdentity {
    /// The VID of `identity`, reachable at `endpoint`.
    ///
    /// # Errors
    ///
    /// [`TspError::NoVid`] when the key manager does not hold the identity's keys.
    pub fn new(
        identity: &ActorIdentity,
        km: Arc<dyn KeyManager>,
        endpoint: Url,
    ) -> Result<Self, TspError> {
        let sig = km
            .public_key(&identity.signing.key_name)
            .map_err(|e| TspError::NoVid(e.to_string()))?;
        let enc = km
            .public_key(&identity.agreement.key_name)
            .and_then(|k| k.to_x25519())
            .map_err(|e| TspError::NoVid(e.to_string()))?;
        Ok(TspIdentity {
            vid: Vid::new(
                identity.did.clone(),
                endpoint,
                VidSignatureKeyType::Ed25519,
                PublicVerificationKeyData::from(sig.bytes.to_vec()),
                VidEncryptionKeyType::X25519,
                PublicKeyData::from(enc.bytes.to_vec()),
            ),
            area: KeyManagerArea(km),
            signing_name: identity.signing.key_name.clone(),
            agreement_name: identity.agreement.key_name.clone(),
        })
    }

    /// The public VID.
    pub fn vid(&self) -> &Vid {
        &self.vid
    }
}

impl VerifiedVid for TspIdentity {
    fn identifier(&self) -> &str {
        self.vid.identifier()
    }
    fn endpoint(&self) -> &Url {
        self.vid.endpoint()
    }
    fn verifying_key(&self) -> &PublicVerificationKeyData {
        self.vid.verifying_key()
    }
    fn encryption_key(&self) -> &PublicKeyData {
        self.vid.encryption_key()
    }
    fn encryption_key_type(&self) -> VidEncryptionKeyType {
        self.vid.encryption_key_type()
    }
    fn signature_key_type(&self) -> VidSignatureKeyType {
        self.vid.signature_key_type()
    }
}

impl PrivateVid for TspIdentity {
    fn secure_area(&self) -> &dyn SecureArea {
        &self.area
    }
    fn signing_key_name(&self) -> &str {
        &self.signing_name
    }
    fn decryption_key_name(&self) -> &str {
        &self.agreement_name
    }
}

/// The relationship of this actor with one peer VID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relationship {
    /// This actor sent a relationship request (`TSP_RFI`) with this digest.
    Requested(Digest),
    /// The peer requested one with this thread id; not yet accepted.
    Incoming(Digest),
    /// Both sides accepted.
    Established,
}

/// A sealed TSP message and where to send it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedTsp {
    /// The recipient's transport endpoint.
    pub endpoint: Url,
    /// The CESR-encoded message.
    pub bytes: Vec<u8>,
}

/// What an opened TSP message carried.
#[derive(Clone, Debug, PartialEq)]
pub enum TspReceived {
    /// An event, with its verified origin.
    Event(Box<OpenedMessage>),
    /// The sender asks for a relationship.
    RelationshipRequested {
        /// The sender VID.
        sender: String,
        /// The thread id to accept.
        thread_id: Digest,
    },
    /// The sender accepted our request.
    RelationshipAccepted {
        /// The sender VID.
        sender: String,
    },
    /// The sender refused or cancelled the relationship.
    RelationshipCancelled {
        /// The sender VID.
        sender: String,
    },
}

/// One actor's TSP endpoint state: its identity and its relationships with peers.
// @tg: implements:: [[openspec:relay-mailboxes#TSP Direct Transport]] {note: "direct-mode sealing, opening and relationship setup"}
pub struct TspSession {
    me: TspIdentity,
    require_relationship: bool,
    relationships: Mutex<HashMap<String, Relationship>>,
}

impl fmt::Debug for TspSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspSession")
            .field("me", &self.me)
            .field("require_relationship", &self.require_relationship)
            .finish()
    }
}

impl TspSession {
    /// A session for `me`. With `require_relationship`, application messages are sent and
    /// accepted only within an established relationship, as TSP endpoints require.
    pub fn new(me: TspIdentity, require_relationship: bool) -> Self {
        TspSession {
            me,
            require_relationship,
            relationships: Mutex::new(HashMap::new()),
        }
    }

    /// This session's identity.
    pub fn identity(&self) -> &TspIdentity {
        &self.me
    }

    /// The relationship with `peer`, if any.
    pub fn relationship(&self, peer: &str) -> Option<Relationship> {
        self.relationships
            .lock()
            .ok()
            .and_then(|r| r.get(peer).copied())
    }

    fn set(&self, peer: &str, r: Relationship) {
        if let Ok(mut m) = self.relationships.lock() {
            m.insert(peer.to_owned(), r);
        }
    }

    fn seal(
        &self,
        peer: &Vid,
        payload: Payload<&[u8]>,
        digest: Option<&mut Digest>,
    ) -> Result<SealedTsp, TspError> {
        let msg = tsp_sdk::crypto::seal_and_hash(&self.me, peer, payload, digest)
            .map_err(|e| TspError::Seal(e.to_string()))?;
        Ok(SealedTsp {
            endpoint: peer.endpoint().clone(),
            bytes: msg,
        })
    }

    /// A relationship request (`TSP_RFI`, direct form) to `peer`.
    ///
    /// # Errors
    ///
    /// [`TspError::Seal`] when sealing fails.
    pub fn request_relationship(&self, peer: &Vid) -> Result<SealedTsp, TspError> {
        let mut digest = Digest::default();
        let sealed = self.seal(
            peer,
            Payload::RequestRelationship {
                thread_id: Digest::default(),
                form: RelationshipForm::Direct,
                reply_path: Vec::new(),
            },
            Some(&mut digest),
        )?;
        self.set(peer.identifier(), Relationship::Requested(digest));
        Ok(sealed)
    }

    /// Accepts the relationship request `thread_id` from `peer` (`TSP_RFA`).
    ///
    /// # Errors
    ///
    /// [`TspError`] when no such request is pending or sealing fails.
    pub fn accept_relationship(
        &self,
        peer: &Vid,
        thread_id: Digest,
    ) -> Result<SealedTsp, TspError> {
        if self.relationship(peer.identifier()) != Some(Relationship::Incoming(thread_id)) {
            return Err(TspError::Unexpected(format!(
                "no pending request from {}",
                peer.identifier()
            )));
        }
        let mut reply = Digest::default();
        let sealed = self.seal(
            peer,
            Payload::AcceptRelationship {
                thread_id,
                reply_thread_id: Digest::default(),
                form: RelationshipForm::Direct,
            },
            Some(&mut reply),
        )?;
        self.set(peer.identifier(), Relationship::Established);
        Ok(sealed)
    }

    /// Refuses the relationship request `thread_id` from `peer` (`TSP_RFD`).
    ///
    /// # Errors
    ///
    /// [`TspError::Seal`] when sealing fails.
    pub fn refuse_relationship(
        &self,
        peer: &Vid,
        thread_id: Digest,
    ) -> Result<SealedTsp, TspError> {
        if let Ok(mut m) = self.relationships.lock() {
            m.remove(peer.identifier());
        }
        self.seal(peer, Payload::CancelRelationship { thread_id }, None)
    }

    /// Seals `envelope` to `peer` as a TSP application message.
    ///
    /// # Errors
    ///
    /// [`TspError::RelationshipRequired`] when this session requires a relationship and none is
    /// established; [`TspError::Seal`] when sealing fails.
    pub fn seal_event(&self, peer: &Vid, envelope: &RelayEnvelope) -> Result<SealedTsp, TspError> {
        if self.require_relationship
            && self.relationship(peer.identifier()) != Some(Relationship::Established)
        {
            return Err(TspError::RelationshipRequired(peer.identifier().to_owned()));
        }
        let body = envelope.to_bytes();
        self.seal(peer, Payload::Content(&body), None)
    }

    /// Sends `envelope` to `peer`, first establishing a relationship when none exists. `exchange`
    /// delivers a sealed message and returns the peer's reply, if any (the transport's
    /// request/response leg).
    ///
    /// # Errors
    ///
    /// [`TspError::RelationshipRefused`] when the peer refuses; transport and sealing failures.
    pub fn send_with_relationship(
        &self,
        peer: &Vid,
        envelope: &RelayEnvelope,
        resolve: &dyn Fn(&str) -> Result<Vid, TspError>,
        exchange: &mut dyn FnMut(SealedTsp) -> Result<Option<Vec<u8>>, TspError>,
    ) -> Result<(), TspError> {
        if self.relationship(peer.identifier()) != Some(Relationship::Established) {
            let rfi = self.request_relationship(peer)?;
            let mut reply = exchange(rfi)?
                .ok_or_else(|| TspError::RelationshipRefused(peer.identifier().to_owned()))?;
            match self.open(&mut reply, resolve)? {
                TspReceived::RelationshipAccepted { .. } => {}
                TspReceived::RelationshipCancelled { sender } => {
                    return Err(TspError::RelationshipRefused(sender));
                }
                other => return Err(TspError::Unexpected(format!("{other:?}"))),
            }
        }
        exchange(self.seal_event(peer, envelope)?)?;
        Ok(())
    }

    /// Seals `envelope` to `peer` for routed mode: the event is sealed to `peer` (only it can
    /// open it) and nested in a routed message to the first intermediary of `hops`, whose route
    /// names the remaining intermediaries and then `peer`.
    ///
    /// # Errors
    ///
    /// [`TspError::Seal`] when sealing fails or `hops` is empty.
    // @tg: implements:: [[openspec:relay-mailboxes#TSP Routed Transport Through Intermediaries]]
    pub fn seal_routed(
        &self,
        peer: &Vid,
        hops: &[Vid],
        envelope: &RelayEnvelope,
    ) -> Result<SealedTsp, TspError> {
        let first = hops
            .first()
            .ok_or_else(|| TspError::Seal("routed mode needs an intermediary".into()))?;
        let body = envelope.to_bytes();
        let inner = tsp_sdk::crypto::seal(&self.me, peer, Payload::Content(&body))
            .map_err(|e| TspError::Seal(e.to_string()))?;
        let mut route: Vec<&[u8]> = hops[1..]
            .iter()
            .map(|v| v.identifier().as_bytes())
            .collect();
        route.push(peer.identifier().as_bytes());
        self.seal(first, Payload::RoutedMessage(route, &inner), None)
    }

    /// Opens the inner message of a routed delivery: sealed by the original sender to this
    /// actor, so the sender is verified by its own signature, not by the intermediary's.
    fn open_nested(
        &self,
        inner: &mut [u8],
        resolve: &dyn Fn(&str) -> Result<Vid, TspError>,
    ) -> Result<TspReceived, TspError> {
        let (sender_id, receiver_id) =
            match tsp_sdk::cesr::probe(inner).map_err(|e| TspError::Open(format!("{e:?}")))? {
                EnvelopeType::EncryptedMessage { sender, receiver } => (
                    String::from_utf8_lossy(sender).into_owned(),
                    String::from_utf8_lossy(receiver).into_owned(),
                ),
                EnvelopeType::SignedMessage { .. } => {
                    return Err(TspError::Open("nested message is not encrypted".into()));
                }
            };
        if receiver_id != self.me.identifier() {
            return Err(TspError::Open(format!("nested message for {receiver_id}")));
        }
        let sender_vid = resolve(&sender_id)?;
        let (payload, _, _) = tsp_sdk::crypto::open(&self.me, &sender_vid, inner)
            .map_err(|e| TspError::Open(e.to_string()))?;
        match payload {
            Payload::Content(body) => {
                let envelope =
                    RelayEnvelope::from_bytes(body).map_err(|e| TspError::Open(e.to_string()))?;
                let key = format!("{sender_id}#tsp-signature");
                Ok(TspReceived::Event(Box::new(OpenedMessage {
                    envelope,
                    origin: VerifiedOrigin::verified(sender_id, key, TSP_PROCESSOR),
                })))
            }
            _ => Err(TspError::Unexpected(
                "nested payload is not an event".into(),
            )),
        }
    }

    /// Opens a TSP message addressed to this actor. The sender's VID comes from `resolve` (its
    /// DID resolved through the directory); the message must be signed by that VID's key.
    ///
    /// # Errors
    ///
    /// [`TspError::Open`] for tampered messages, messages for another VID, or senders whose keys
    /// do not authenticate the message; [`TspError::RelationshipRequired`] for application
    /// messages outside a required relationship.
    pub fn open(
        &self,
        bytes: &mut [u8],
        resolve: &dyn Fn(&str) -> Result<Vid, TspError>,
    ) -> Result<TspReceived, TspError> {
        let (sender_id, receiver_id) =
            match tsp_sdk::cesr::probe(bytes).map_err(|e| TspError::Open(format!("{e:?}")))? {
                EnvelopeType::EncryptedMessage { sender, receiver } => (
                    String::from_utf8_lossy(sender).into_owned(),
                    String::from_utf8_lossy(receiver).into_owned(),
                ),
                EnvelopeType::SignedMessage { .. } => {
                    return Err(TspError::Open("not an encrypted message".into()));
                }
            };
        if receiver_id != self.me.identifier() {
            return Err(TspError::Open(format!("addressed to {receiver_id}")));
        }
        let sender_vid = resolve(&sender_id)?;
        if sender_vid.identifier() != sender_id {
            return Err(TspError::Open(format!(
                "resolved {} for sender {sender_id}",
                sender_vid.identifier()
            )));
        }
        let (payload, _, _) = tsp_sdk::crypto::open(&self.me, &sender_vid, bytes)
            .map_err(|e| TspError::Open(e.to_string()))?;
        match payload {
            Payload::Content(body) => {
                if self.require_relationship
                    && self.relationship(&sender_id) != Some(Relationship::Established)
                {
                    return Err(TspError::RelationshipRequired(sender_id));
                }
                let envelope =
                    RelayEnvelope::from_bytes(body).map_err(|e| TspError::Open(e.to_string()))?;
                let key = format!("{sender_id}#tsp-signature");
                Ok(TspReceived::Event(Box::new(OpenedMessage {
                    envelope,
                    origin: VerifiedOrigin::verified(sender_id, key, TSP_PROCESSOR),
                })))
            }
            Payload::RequestRelationship { thread_id, .. } => {
                self.set(&sender_id, Relationship::Incoming(thread_id));
                Ok(TspReceived::RelationshipRequested {
                    sender: sender_id,
                    thread_id,
                })
            }
            Payload::AcceptRelationship { thread_id, .. } => match self.relationship(&sender_id) {
                Some(Relationship::Requested(d)) if d == thread_id => {
                    self.set(&sender_id, Relationship::Established);
                    Ok(TspReceived::RelationshipAccepted { sender: sender_id })
                }
                _ => Err(TspError::Unexpected(format!(
                    "accept from {sender_id} answers no request"
                ))),
            },
            Payload::CancelRelationship { .. } => {
                if let Ok(mut m) = self.relationships.lock() {
                    m.remove(&sender_id);
                }
                Ok(TspReceived::RelationshipCancelled { sender: sender_id })
            }
            // Routed mode: the last intermediary delivered the sender's inner message.
            Payload::NestedMessage(inner) => self.open_nested(inner, resolve),
            _ => Err(TspError::Unexpected("unsupported TSP payload".into())),
        }
    }
}

/// The raw `TSPTransport` endpoint URI of a document, as written.
pub fn tsp_uri(doc: &DidDocument) -> Option<String> {
    doc.service
        .iter()
        .filter(|s| s.has_type("TSPTransport") || s.has_type("TspTransport"))
        .flat_map(|s| s.endpoints.iter())
        .map(|e| e.uri.clone())
        .next()
}

/// The VID of `doc` reachable at `uri` (one of its `TSPTransport` endpoints).
///
/// # Errors
///
/// [`TspError::NoVid`] when a key is missing or `uri` is not a URL.
pub fn vid_at(doc: &DidDocument, uri: &str) -> Result<Vid, TspError> {
    let v = vid_from_document(doc)?;
    let url = Url::parse(uri).map_err(|e| TspError::NoVid(format!("{uri}: {e}")))?;
    Ok(Vid::new(
        v.identifier().to_owned(),
        url,
        VidSignatureKeyType::Ed25519,
        v.verifying_key().clone(),
        VidEncryptionKeyType::X25519,
        v.encryption_key().clone(),
    ))
}

fn resolve_vid(resolver: &dyn DidResolver, id: &str) -> Result<Vid, TspError> {
    let did = Did::parse(id).map_err(|e| TspError::NoVid(e.to_string()))?;
    let doc = resolver
        .resolve(&did)
        .map_err(|e| TspError::NoVid(e.to_string()))?
        .document;
    vid_from_document(&doc)
}

fn probe_ids(bytes: &mut [u8]) -> Option<(String, String)> {
    match tsp_sdk::cesr::probe(bytes).ok()? {
        EnvelopeType::EncryptedMessage { sender, receiver } => Some((
            String::from_utf8_lossy(sender).into_owned(),
            String::from_utf8_lossy(receiver).into_owned(),
        )),
        EnvelopeType::SignedMessage { .. } => None,
    }
}

/// TSP as a relay protocol: direct mode with relationship setup before the first event when
/// the recipient's `TSPTransport` service has no intermediaries, routed mode through the
/// intermediaries its `routingKeys` name otherwise. One session per local actor keeps its
/// relationships.
// @tg: implements:: [[openspec:relay-mailboxes#TSP Direct Transport]]
// @tg: implements:: [[openspec:relay-mailboxes#TSP Routed Transport Through Intermediaries]]
// @tg: implements:: [[openspec:relay-mailboxes#End-To-End Encryption Through Relays]]
pub struct TspProtocol {
    sessions: Mutex<HashMap<String, Arc<TspSession>>>,
    require_relationship: bool,
    refuse: Mutex<std::collections::BTreeSet<String>>,
}

impl fmt::Debug for TspProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspProtocol")
            .field("require_relationship", &self.require_relationship)
            .finish_non_exhaustive()
    }
}

impl TspProtocol {
    /// A TSP protocol whose local endpoints require (`true`) or do not require a relationship
    /// before application messages in direct mode.
    pub fn new(require_relationship: bool) -> Self {
        TspProtocol {
            sessions: Mutex::new(HashMap::new()),
            require_relationship,
            refuse: Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    /// Makes the local endpoints refuse relationship requests from `sender`.
    pub fn refuse_relationships_from(&self, sender: &str) {
        if let Ok(mut r) = self.refuse.lock() {
            r.insert(sender.to_owned());
        }
    }

    /// The session of the local actor `actor`, created on first use. Its VID's endpoint is the
    /// actor's own `TSPTransport` service (or a placeholder when it publishes none).
    ///
    /// # Errors
    ///
    /// [`TspError::NoVid`] when the key manager lacks the actor's keys.
    pub fn session(&self, actor: &LocalActor) -> Result<Arc<TspSession>, TspError> {
        let mut s = self
            .sessions
            .lock()
            .map_err(|_| TspError::Unexpected("session lock".into()))?;
        if let Some(x) = s.get(&actor.identity.did) {
            return Ok(x.clone());
        }
        let uri = actor
            .identity
            .services
            .iter()
            .filter(|s| s.has_type("TSPTransport") || s.has_type("TspTransport"))
            .flat_map(|s| s.endpoints.iter())
            .find_map(|e| Url::parse(&e.uri).ok())
            .or_else(|| Url::parse("tsp://unpublished").ok())
            .ok_or_else(|| TspError::NoVid("endpoint".into()))?;
        let session = Arc::new(TspSession::new(
            TspIdentity::new(&actor.identity, actor.km.clone(), uri)?,
            self.require_relationship,
        ));
        s.insert(actor.identity.did.clone(), session.clone());
        Ok(session)
    }

    /// The endpoint at which this node receives direct TSP messages for its actors: relationship
    /// requests are answered at once, events are stored in `inbound` for the actor's mailbox to
    /// pick up.
    pub fn endpoint(
        self: &Arc<Self>,
        identities: Arc<crate::federation::Identities>,
        inbound: Arc<DropBox>,
        resolver: Arc<dyn DidResolver>,
    ) -> TspInbound {
        TspInbound {
            protocol: self.clone(),
            identities,
            inbound,
            resolver,
        }
    }
}

impl RelayProtocol for TspProtocol {
    fn kind(&self) -> TransportKind {
        TransportKind::Tsp
    }

    fn send(&self, cx: &SendContext<'_>) -> Result<(), SendError> {
        let Some(from) = cx.from else {
            return Err(SendError::Unsupported(
                "TSP needs a sender VID; the sender has no DID".into(),
            ));
        };
        let session = self
            .session(from)
            .map_err(|e| SendError::Failed(CommunicationFailure::SealFailed(e.to_string())))?;
        let peer = vid_at(&cx.to.document, &cx.route.endpoint.uri)
            .map_err(|_| SendError::Failed(CommunicationFailure::NoCompatibleEndpoint))?;
        let resolver = cx.resolver;
        let resolve = |id: &str| resolve_vid(resolver, id);
        if cx.route.endpoint.routing_keys.is_empty() {
            let mut net_error = None;
            let uri = cx.route.endpoint.uri.as_str();
            let r = session.send_with_relationship(
                &peer,
                cx.envelope,
                &resolve,
                &mut |sealed: SealedTsp| match cx.net.post(uri, &sealed.bytes) {
                    Ok(resp) => Ok((!resp.is_empty()).then_some(resp)),
                    Err(e) => {
                        net_error = Some(e);
                        Err(TspError::Unexpected("network".into()))
                    }
                },
            );
            return match (r, net_error) {
                (Ok(()), _) => Ok(()),
                (Err(_), Some(e)) => Err(e.into()),
                (Err(TspError::RelationshipRefused(m)), None) => Err(SendError::Refused(format!(
                    "TSP relationship refused by {m}"
                ))),
                (Err(e), None) => Err(SendError::Failed(CommunicationError::from(e).failure)),
            };
        }
        let mut hops = Vec::new();
        let mut first_uri = None;
        for id in &cx.route.endpoint.routing_keys {
            let did = Did::parse(id).map_err(|e| SendError::Stale(e.to_string()))?;
            let doc = resolver
                .resolve(&did)
                .map_err(|e| SendError::Stale(format!("intermediary {id}: {e}")))?
                .document;
            if first_uri.is_none() {
                first_uri = tsp_uri(&doc);
            }
            hops.push(vid_from_document(&doc).map_err(|e| SendError::Stale(e.to_string()))?);
        }
        let sealed = session
            .seal_routed(&peer, &hops, cx.envelope)
            .map_err(|e| SendError::Failed(CommunicationFailure::SealFailed(e.to_string())))?;
        let uri = first_uri.ok_or_else(|| SendError::Stale("intermediary endpoint".into()))?;
        cx.net.post(&uri, &sealed.bytes)?;
        Ok(())
    }

    fn open(
        &self,
        to: &LocalActor,
        bytes: &[u8],
        resolver: &dyn DidResolver,
        _now: Timestamp,
    ) -> Result<OpenedMessage, Refusal> {
        let session = self
            .session(to)
            .map_err(|e| Refusal::MalformedEnvelope(e.to_string()))?;
        let mut b = bytes.to_vec();
        let claimed = probe_ids(&mut b.clone()).map(|(s, _)| s);
        match session.open(&mut b, &|id: &str| resolve_vid(resolver, id)) {
            Ok(TspReceived::Event(m)) => Ok(*m),
            Ok(other) => Err(Refusal::MalformedEnvelope(format!(
                "not an event: {other:?}"
            ))),
            Err(TspError::Open(reason)) => Err(Refusal::Undecryptable {
                reason,
                kid: None,
                sender: claimed,
            }),
            Err(e) => Err(Refusal::MalformedEnvelope(e.to_string())),
        }
    }
}

/// A node's TSP endpoint (see [`TspProtocol::endpoint`]).
pub struct TspInbound {
    protocol: Arc<TspProtocol>,
    identities: Arc<crate::federation::Identities>,
    inbound: Arc<DropBox>,
    resolver: Arc<dyn DidResolver>,
}

impl fmt::Debug for TspInbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TspInbound").finish_non_exhaustive()
    }
}

impl RelayEndpoint for TspInbound {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        let mut copy = body.to_vec();
        let (_, receiver) =
            probe_ids(&mut copy.clone()).ok_or_else(|| NetError::Refused("not TSP".into()))?;
        let actor = self
            .identities
            .actor(&receiver)
            .ok_or_else(|| NetError::Refused(format!("no VID {receiver} here")))?;
        let session = self
            .protocol
            .session(&actor)
            .map_err(|e| NetError::Refused(e.to_string()))?;
        let resolver = self.resolver.as_ref();
        let resolve = |id: &str| resolve_vid(resolver, id);
        match session.open(&mut copy, &resolve) {
            Ok(TspReceived::RelationshipRequested { sender, thread_id }) => {
                let peer = resolve(&sender).map_err(|e| NetError::Refused(e.to_string()))?;
                let refused = self
                    .protocol
                    .refuse
                    .lock()
                    .is_ok_and(|r| r.contains(&sender));
                let reply = if refused {
                    session.refuse_relationship(&peer, thread_id)
                } else {
                    session.accept_relationship(&peer, thread_id)
                };
                reply
                    .map(|s| s.bytes)
                    .map_err(|e| NetError::Refused(e.to_string()))
            }
            Ok(TspReceived::Event(_)) => {
                self.inbound.store(&receiver, body.to_vec());
                Ok(Vec::new())
            }
            Ok(_) => Ok(Vec::new()),
            Err(e) => Err(NetError::Refused(e.to_string())),
        }
    }
}

/// One hop of routed mode at an intermediary: where the message goes next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedHop {
    /// The next VID (another intermediary, or the final recipient).
    pub next: String,
    /// True when `next` is the final recipient.
    pub last: bool,
    /// The re-sealed message for `next`.
    pub bytes: Vec<u8>,
}

/// What an intermediary does with a routed message addressed to it (`me`): open its own layer,
/// read the next hop, and re-seal the still-opaque inner message to it, as a routed message to
/// the next intermediary or as a nested message to the final recipient. The intermediary never
/// sees the event.
///
/// # Errors
///
/// [`TspError`] when the message does not open or is not a routed message.
pub fn forward_routed(
    me: &TspIdentity,
    bytes: &mut [u8],
    resolver: &dyn DidResolver,
) -> Result<RoutedHop, TspError> {
    let (sender, receiver) =
        probe_ids(bytes).ok_or_else(|| TspError::Open("not an encrypted message".into()))?;
    if receiver != me.identifier() {
        return Err(TspError::Open(format!("addressed to {receiver}")));
    }
    let sender_vid = resolve_vid(resolver, &sender)?;
    let (payload, _, _) =
        tsp_sdk::crypto::open(me, &sender_vid, bytes).map_err(|e| TspError::Open(e.to_string()))?;
    let Payload::RoutedMessage(hops, inner) = payload else {
        return Err(TspError::Unexpected("not a routed message".into()));
    };
    let next = String::from_utf8_lossy(
        hops.first()
            .ok_or_else(|| TspError::Unexpected("empty route".into()))?,
    )
    .into_owned();
    let next_vid = resolve_vid(resolver, &next)?;
    let rest: Vec<&[u8]> = hops[1..].to_vec();
    let last = rest.is_empty();
    let payload = if last {
        Payload::NestedMessage(inner)
    } else {
        Payload::RoutedMessage(rest, inner)
    };
    let bytes =
        tsp_sdk::crypto::seal(me, &next_vid, payload).map_err(|e| TspError::Seal(e.to_string()))?;
    Ok(RoutedHop { next, last, bytes })
}
