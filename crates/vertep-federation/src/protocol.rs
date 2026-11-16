//! Protocol spaces (Kolyada, M10) over the federation layer: the agent's join policy, pairwise
//! sealed fan-out through the existing relays and mailboxes, and a participant that journals
//! only what its policy and the conformance check accept.
//!
//! The protocol core is `vertep-kolyada`: documents, the signed envelope and the pure check of
//! an [`Instance`]. This module puts it on the wire.
//!
//! - **Keys and DIDs.** [`ActorSigner`] signs protocol messages with a local actor's key through
//!   its [`KeyManager`](crate::KeyManager); [`ResolverVerifier`] checks a signature against the
//!   sender's resolved DID document.
//! - **Join policy.** A [`JoinPolicy`] decides, before anything is journaled, whether the agent
//!   takes part in an instance a genesis opens: which protocols, which of its roles, which
//!   initiators and, optionally, which credentials. [`JoinPolicy::default`] refuses every
//!   genesis.
//! - **Fan-out.** [`SpaceTransport::fan_out`] sends one copy of a signed message per recipient
//!   DID, each sealed to that recipient alone by the relay protocol of a route its DID document
//!   lists (TSP or DIDComm), or handed over in process through a [`LocalChannel`]. There is no
//!   group key: a mailbox or relay only ever holds copies sealed to its owner, and a message
//!   sealed to some roles never reaches the others. They see it as a hash in the dependencies
//!   of later messages, which [`Instance::observe_hidden`] takes in.
//! - **The participant.** A [`SpaceMember`] picks up its copies from a mailbox, opens them, runs
//!   the policy on a genesis and the conformance check on everything else, journals what passes
//!   and acknowledges a copy only after its journal append or its recorded refusal.
//!
//! A protocol message travels inside a [`RelayEnvelope`] named [`KOLYADA_EVENT`] whose data is
//! the signed message's JSON text, so no transport touches the bytes the signature covers.
//!
//! See `tg.md/views/kolyada.md` for the architecture.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value as Json;
use vertep_core::datamodel::{Event as CoreEvent, EventType, Value};
use vertep_kolyada::{
    Equivocation, Event, Instance, Message, MessageHash, Protocol, Reason, SendError, SignError,
    Signer, Verifier, VerifyError, control,
};

use crate::did::Did;
use crate::directory::{DidDirectory, TransportKind};
use crate::envelope::RelayEnvelope;
use crate::net::{PickupSource, RelayNetwork};
use crate::relay::{LocalActor, RelayProtocol, SendContext, SendError as RelaySendError};
use crate::resolver::DidResolver;
use crate::time::Timestamp;

/// The event name of a relay envelope that carries a protocol message.
pub const KOLYADA_EVENT: &str = "kolyada.message";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------------
// Keys and DIDs
// ---------------------------------------------------------------------------------------------

/// Signs protocol messages as a local DID actor, with its signing key, through its key manager.
#[derive(Clone, Debug)]
pub struct ActorSigner(pub LocalActor);

impl Signer for ActorSigner {
    fn did(&self) -> &str {
        &self.0.identity.did
    }

    fn key_id(&self) -> &str {
        &self.0.identity.signing.kid
    }

    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignError> {
        self.0
            .km
            .sign(&self.0.identity.signing.key_name, data)
            .map_err(|e| SignError(e.to_string()))
    }
}

/// Checks protocol signatures against the sender's DID document: the key must be one of the
/// document's verification methods listed under `authentication` or `assertionMethod`.
#[derive(Clone)]
pub struct ResolverVerifier(pub Arc<dyn DidResolver>);

impl fmt::Debug for ResolverVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResolverVerifier")
    }
}

impl Verifier for ResolverVerifier {
    fn verify(
        &self,
        did: &str,
        key_id: &str,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), VerifyError> {
        let parsed = Did::parse(did).map_err(|e| VerifyError(e.to_string()))?;
        let doc = self
            .0
            .resolve(&parsed)
            .map_err(|e| VerifyError(e.to_string()))?
            .document;
        let method = doc
            .method(key_id)
            .ok_or_else(|| VerifyError(format!("{did} has no key {key_id}")))?;
        let may_sign = doc.authenticates_with(&method.id)
            || doc.assertion_methods().iter().any(|m| m.id == method.id);
        if !may_sign {
            return Err(VerifyError(format!(
                "{key_id} is not a signing key of {did}"
            )));
        }
        if method.public_key.verify(data, signature) {
            Ok(())
        } else {
            Err(VerifyError(format!(
                "the signature does not verify under {key_id}"
            )))
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Join policy
// ---------------------------------------------------------------------------------------------

/// An optional credential condition of a [`JoinRule`]: whether the agent accepts the initiator
/// of a genesis on the strength of credentials it holds or was shown.
pub trait CredentialCheck: Send + Sync {
    /// True when the genesis's initiator presents what the rule asks for.
    fn admits(&self, genesis: &Message) -> bool;
}

impl<F: Fn(&Message) -> bool + Send + Sync> CredentialCheck for F {
    fn admits(&self, genesis: &Message) -> bool {
        self(genesis)
    }
}

/// One rule of a [`JoinPolicy`]: a protocol, and optionally the agent's role in it, the
/// initiators it accepts and a credential condition.
#[derive(Clone)]
pub struct JoinRule {
    protocol: String,
    roles: Option<BTreeSet<String>>,
    initiators: Option<BTreeSet<String>>,
    credential: Option<Arc<dyn CredentialCheck>>,
}

impl fmt::Debug for JoinRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinRule")
            .field("protocol", &self.protocol)
            .field("roles", &self.roles)
            .field("initiators", &self.initiators)
            .field("credential", &self.credential.is_some())
            .finish()
    }
}

impl JoinRule {
    /// Admits the protocol `protocol` (its identity, `urn:vertep:protocol:sha256:…`) in any
    /// role, from any initiator. Narrow it with the other methods.
    pub fn protocol(protocol: impl Into<String>) -> Self {
        JoinRule {
            protocol: protocol.into(),
            roles: None,
            initiators: None,
            credential: None,
        }
    }

    /// Only when the agent is bound to `role` (callable more than once).
    #[must_use]
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.roles
            .get_or_insert_with(BTreeSet::new)
            .insert(role.into());
        self
    }

    /// Only when the genesis is signed by `did` (callable more than once).
    #[must_use]
    pub fn initiator(mut self, did: impl Into<String>) -> Self {
        self.initiators
            .get_or_insert_with(BTreeSet::new)
            .insert(did.into());
        self
    }

    /// Only when `check` admits the genesis.
    #[must_use]
    pub fn credential(mut self, check: Arc<dyn CredentialCheck>) -> Self {
        self.credential = Some(check);
        self
    }

    fn admits(&self, genesis: &Message, local_roles: &[String]) -> Result<(), JoinRefusal> {
        let b = &genesis.body;
        if b.protocol != self.protocol {
            return Err(JoinRefusal::Protocol(b.protocol.clone()));
        }
        if let Some(roles) = &self.roles
            && !local_roles.iter().any(|r| roles.contains(r))
        {
            return Err(JoinRefusal::Role(local_roles.join(", ")));
        }
        if let Some(initiators) = &self.initiators
            && !initiators.contains(&b.sender)
        {
            return Err(JoinRefusal::Initiator(b.sender.clone()));
        }
        if let Some(check) = &self.credential
            && !check.admits(genesis)
        {
            return Err(JoinRefusal::Credential(b.sender.clone()));
        }
        Ok(())
    }
}

/// Why a [`JoinPolicy`] refused a genesis. When no rule admits it, the refusal of the last rule
/// that named its protocol is given, or [`JoinRefusal::Protocol`] when none did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinRefusal {
    /// The message is not a genesis.
    NotAGenesis,
    /// The policy has no rules: the default refuses every genesis.
    Default,
    /// The genesis does not bind the agent's DID to any role.
    NotBound,
    /// No rule admits this protocol.
    Protocol(String),
    /// No rule admits the agent's roles.
    Role(String),
    /// No rule admits this initiator.
    Initiator(String),
    /// The initiator lacks the credential a rule asks for.
    Credential(String),
}

impl fmt::Display for JoinRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAGenesis => f.write_str("not a genesis"),
            Self::Default => f.write_str("the default join policy refuses every genesis"),
            Self::NotBound => f.write_str("the genesis does not bind this agent"),
            Self::Protocol(p) => write!(f, "no rule admits protocol {p}"),
            Self::Role(r) => write!(f, "no rule admits the role {r}"),
            Self::Initiator(d) => write!(f, "no rule admits initiator {d}"),
            Self::Credential(d) => write!(f, "initiator {d} lacks the credential asked for"),
        }
    }
}

impl std::error::Error for JoinRefusal {}

/// An agent's own rules for taking part in protocol instances. A genesis is admitted when one
/// rule admits it. The default policy has no rules and refuses every genesis.
#[derive(Clone, Debug, Default)]
pub struct JoinPolicy {
    rules: Vec<JoinRule>,
}

impl JoinPolicy {
    /// The policy that refuses every genesis, the same as [`JoinPolicy::default`].
    pub fn refuse_all() -> Self {
        Self::default()
    }

    /// The policy with `rule` added.
    #[must_use]
    pub fn allow(mut self, rule: JoinRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Decides on `genesis` for the agent `local_did`. It reads the genesis only, verifies
    /// nothing and changes nothing, so it runs before the signature check and the journal.
    ///
    /// # Errors
    ///
    /// The [`JoinRefusal`].
    // @tg: implements:: [[openspec:protocol-spaces#Join Policy]]
    pub fn decide(&self, genesis: &Message, local_did: &str) -> Result<(), JoinRefusal> {
        let b = &genesis.body;
        if b.kind != control::GENESIS {
            return Err(JoinRefusal::NotAGenesis);
        }
        let local_roles: Vec<String> = b
            .payload
            .get("bindings")
            .and_then(Json::as_object)
            .map(|m| {
                m.iter()
                    .filter(|(_, d)| d.as_str() == Some(local_did))
                    .map(|(r, _)| r.clone())
                    .collect()
            })
            .unwrap_or_default();
        if local_roles.is_empty() {
            return Err(JoinRefusal::NotBound);
        }
        if self.rules.is_empty() {
            return Err(JoinRefusal::Default);
        }
        let mut refusal = JoinRefusal::Protocol(b.protocol.clone());
        for rule in &self.rules {
            match rule.admits(genesis, &local_roles) {
                Ok(()) => return Ok(()),
                Err(JoinRefusal::Protocol(_)) => {}
                Err(e) => refusal = e,
            }
        }
        Err(refusal)
    }
}

// ---------------------------------------------------------------------------------------------
// Carrying a message
// ---------------------------------------------------------------------------------------------

/// The relay envelope that carries `msg`: event [`KOLYADA_EVENT`], message id the message hash,
/// data the signed message's JSON text.
pub fn envelope_of(msg: &Message) -> RelayEnvelope {
    let mut e = CoreEvent::named(KOLYADA_EVENT, EventType::External);
    e.data = Value::String(msg.to_json());
    RelayEnvelope::from_event(msg.hash(), &e)
}

/// The protocol message a relay envelope carries.
///
/// # Errors
///
/// A text naming why the envelope carries no protocol message.
pub fn message_of(envelope: &RelayEnvelope) -> Result<Message, String> {
    if envelope.name != KOLYADA_EVENT {
        return Err(format!("event {} is not a protocol message", envelope.name));
    }
    let Value::String(text) = &envelope.data.0 else {
        return Err("the protocol message is not JSON text".into());
    };
    Message::from_json(text).map_err(|e| e.to_string())
}

/// An in-process channel between participants on one node. A copy is the signed message's JSON
/// text, queued for one DID; nothing leaves the process, so nothing is sealed.
#[derive(Debug, Default)]
pub struct LocalChannel {
    queues: Mutex<BTreeMap<String, VecDeque<Vec<u8>>>>,
}

impl LocalChannel {
    /// An empty channel.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Makes `did` reachable through this channel.
    pub fn host(&self, did: &str) {
        lock(&self.queues).entry(did.to_owned()).or_default();
    }

    /// True when `did` is reachable through this channel.
    pub fn hosts(&self, did: &str) -> bool {
        lock(&self.queues).contains_key(did)
    }

    /// Queues `msg` for `did`; false when the channel does not host `did`.
    pub fn post(&self, did: &str, msg: &Message) -> bool {
        match lock(&self.queues).get_mut(did) {
            Some(q) => {
                q.push_back(msg.to_json().into_bytes());
                true
            }
            None => false,
        }
    }

    /// The copies queued for `did`, oldest first, as JSON bytes.
    pub fn held_for(&self, did: &str) -> Vec<Vec<u8>> {
        lock(&self.queues)
            .get(did)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn take(&self, did: &str) -> Vec<Vec<u8>> {
        lock(&self.queues)
            .get_mut(did)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default()
    }

    fn requeue(&self, did: &str, bytes: Vec<Vec<u8>>) {
        if bytes.is_empty() {
            return;
        }
        let mut q = lock(&self.queues);
        if let Some(queue) = q.get_mut(did) {
            for b in bytes.into_iter().rev() {
                queue.push_front(b);
            }
        }
    }
}

/// How a copy left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// Sealed to the recipient by a relay protocol.
    Relay(TransportKind),
    /// Through a [`LocalChannel`].
    Local,
}

/// One copy of a fanned-out message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    /// The recipient DID.
    pub did: String,
    /// The roles it is bound to among the message's recipients.
    pub roles: Vec<String>,
    /// How it left.
    pub via: Via,
}

/// What [`SpaceTransport::fan_out`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FanOut {
    /// The copies that left, one per recipient DID.
    pub sent: Vec<Copy>,
    /// The recipient DIDs no route reached, with the last failure.
    pub failed: Vec<(String, String)>,
}

impl FanOut {
    /// True when every recipient got its copy.
    pub fn complete(&self) -> bool {
        self.failed.is_empty()
    }
}

/// Sends protocol messages to the members of a space: one copy per recipient DID, each sealed
/// to that recipient alone, over the routes its DID document lists, through the relay protocols
/// and network the federation layer already uses.
pub struct SpaceTransport {
    directory: DidDirectory,
    resolver: Arc<dyn DidResolver>,
    net: Arc<dyn RelayNetwork>,
    protocols: Vec<Arc<dyn RelayProtocol>>,
    local: Option<Arc<LocalChannel>>,
}

impl fmt::Debug for SpaceTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpaceTransport")
            .field("directory", &self.directory)
            .field("local", &self.local.is_some())
            .finish_non_exhaustive()
    }
}

impl SpaceTransport {
    /// A transport resolving DIDs through `resolver` and posting on `net`, with no relay
    /// protocol yet; add them with [`SpaceTransport::protocol`].
    pub fn new(resolver: Arc<dyn DidResolver>, net: Arc<dyn RelayNetwork>) -> Self {
        SpaceTransport {
            directory: DidDirectory::new(resolver.clone()),
            resolver,
            net,
            protocols: Vec::new(),
            local: None,
        }
    }

    /// Adds a relay protocol (TSP, DIDComm).
    #[must_use]
    pub fn protocol(mut self, p: Arc<dyn RelayProtocol>) -> Self {
        self.protocols.push(p);
        self
    }

    /// Tries transports in `preference` order (TSP, then DIDComm, by default).
    #[must_use]
    pub fn preference(mut self, preference: Vec<TransportKind>) -> Self {
        self.directory = self.directory.with_preference(preference);
        self
    }

    /// Delivers to the DIDs `channel` hosts through it, before any relay.
    #[must_use]
    pub fn local(mut self, channel: Arc<LocalChannel>) -> Self {
        self.local = Some(channel);
        self
    }

    /// The recipient DIDs of `msg` under `bindings` (role to DID), each with its roles, without
    /// the sender's own DID.
    pub fn recipients(
        msg: &Message,
        bindings: &BTreeMap<String, String>,
    ) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for role in &msg.body.recipients {
            if let Some(did) = bindings.get(role)
                && *did != msg.body.sender
            {
                out.entry(did.clone()).or_default().push(role.clone());
            }
        }
        out
    }

    /// Sends `msg`, signed by `from`, to every recipient DID of it under `bindings`: one copy
    /// each, sealed to that recipient by the first route of its DID document that works (a
    /// route that is unreachable, stale or unusable falls through to the next). Nothing is sent
    /// to a DID outside the message's recipient set.
    // @tg: implements:: [[openspec:protocol-spaces#Pairwise Sealed Fan-Out]]
    pub fn fan_out(
        &self,
        from: &LocalActor,
        msg: &Message,
        bindings: &BTreeMap<String, String>,
    ) -> FanOut {
        let envelope = envelope_of(msg);
        let mut report = FanOut::default();
        for (did, roles) in Self::recipients(msg, bindings) {
            if self.local.as_ref().is_some_and(|c| c.post(&did, msg)) {
                report.sent.push(Copy {
                    did,
                    roles,
                    via: Via::Local,
                });
                continue;
            }
            match self.send_one(from, &did, &envelope) {
                Ok(kind) => report.sent.push(Copy {
                    did,
                    roles,
                    via: Via::Relay(kind),
                }),
                Err(e) => report.failed.push((did, e)),
            }
        }
        report
    }

    fn send_one(
        &self,
        from: &LocalActor,
        did: &str,
        envelope: &RelayEnvelope,
    ) -> Result<TransportKind, String> {
        let to = self.directory.resolve(did).map_err(|e| e.to_string())?;
        // A TSP peer resolves the sender's VID from its DID document, so a sender that publishes
        // no TSP service cannot use a TSP route.
        let sender_has_tsp = from.identity.services.iter().any(|s| {
            TransportKind::Tsp
                .service_types()
                .iter()
                .any(|t| s.has_type(t))
        });
        let mut last = String::from("no route with a known relay protocol");
        for route in &to.routes {
            if route.transport == TransportKind::Tsp && !sender_has_tsp {
                last = String::from("the sender publishes no TSP service");
                continue;
            }
            let Some(p) = self.protocols.iter().find(|p| p.kind() == route.transport) else {
                continue;
            };
            let cx = SendContext {
                from: Some(from),
                to: &to,
                route,
                envelope,
                net: self.net.as_ref(),
                resolver: self.resolver.as_ref(),
            };
            match p.send(&cx) {
                Ok(()) => return Ok(route.transport),
                Err(
                    e @ (RelaySendError::Unreachable(_)
                    | RelaySendError::Stale(_)
                    | RelaySendError::Unsupported(_)),
                ) => last = e.to_string(),
                Err(e) => return Err(e.to_string()),
            }
        }
        Err(last)
    }
}

// ---------------------------------------------------------------------------------------------
// The participant
// ---------------------------------------------------------------------------------------------

/// Why a participant refused a message. Nothing refused is journaled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The join policy refused a genesis.
    Policy(JoinRefusal),
    /// The genesis names a protocol document the participant does not hold.
    UnknownProtocol(String),
    /// The conformance check refused it.
    Check(Reason),
    /// The copy could not be opened or carries no protocol message.
    Transport(String),
    /// It belongs to an instance whose genesis this participant refused.
    RefusedInstance(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(r) => write!(f, "join policy: {r}"),
            Self::UnknownProtocol(p) => write!(f, "unknown protocol {p}"),
            Self::Check(r) => write!(f, "refused: {r}"),
            Self::Transport(t) => write!(f, "undeliverable: {t}"),
            Self::RefusedInstance(i) => write!(f, "instance {i} was refused"),
        }
    }
}

/// What a participant did with one message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Journaled: a genesis the policy admitted, or a message the check accepted.
    Journaled {
        /// The instance id.
        instance: String,
        /// The message hash.
        hash: MessageHash,
    },
    /// Already journaled; dropped.
    Duplicate(MessageHash),
    /// Waits for its dependencies; its copy stays unacknowledged.
    Held(MessageHash),
    /// Names an instance whose genesis has not arrived (relays keep no order across each
    /// other); its copy stays unacknowledged and is taken in again on a later pickup.
    Deferred(MessageHash),
    /// Refused and recorded; never journaled.
    Refused {
        /// The message hash, when the copy opened.
        hash: Option<MessageHash>,
        /// Why.
        refusal: Refusal,
    },
    /// A held message waited past the protocol's timeout.
    Stalled {
        /// The message hash.
        hash: MessageHash,
        /// The dependencies it still lacks.
        missing: Vec<MessageHash>,
    },
    /// Two different signed messages with one role and sequence number.
    Equivocation(Box<Equivocation>),
}

impl Outcome {
    /// The message hash this outcome is about, if any.
    fn hash(&self) -> Option<&str> {
        match self {
            Self::Journaled { hash, .. }
            | Self::Duplicate(hash)
            | Self::Held(hash)
            | Self::Deferred(hash)
            | Self::Stalled { hash, .. } => Some(hash),
            Self::Refused { hash, .. } => hash.as_deref(),
            Self::Equivocation(_) => None,
        }
    }

    /// True when the copy may be acknowledged: the message was journaled, was a duplicate, or
    /// its refusal or stall is recorded.
    fn settles(&self) -> bool {
        !matches!(
            self,
            Self::Held(_) | Self::Deferred(_) | Self::Equivocation(_)
        )
    }
}

/// One journal entry: a genesis the policy admitted, or a message the check accepted, with the
/// instance it belongs to. A participant journals nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// The instance id.
    pub instance: String,
    /// The signed message.
    pub message: Message,
}

/// One agent's side of its protocol spaces: the documents it knows, its join policy, its
/// instances and its journal.
pub struct SpaceMember {
    did: String,
    signer: Arc<dyn Signer + Send + Sync>,
    verifier: Arc<dyn Verifier + Send + Sync>,
    policy: JoinPolicy,
    protocols: BTreeMap<String, Arc<Protocol>>,
    instances: BTreeMap<String, Instance>,
    journal: Vec<JournalEntry>,
    refusals: Vec<(Option<MessageHash>, Refusal)>,
    refused_instances: BTreeSet<String>,
    unacked: BTreeMap<MessageHash, Vec<String>>,
}

impl fmt::Debug for SpaceMember {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpaceMember")
            .field("did", &self.did)
            .field("policy", &self.policy)
            .field("instances", &self.instances.keys().collect::<Vec<_>>())
            .field("journal", &self.journal.len())
            .finish_non_exhaustive()
    }
}

impl SpaceMember {
    /// A participant signing with `signer` (its DID is the signer's) and verifying with
    /// `verifier`, under the default join policy, which refuses every genesis.
    pub fn new(
        signer: Arc<dyn Signer + Send + Sync>,
        verifier: Arc<dyn Verifier + Send + Sync>,
    ) -> Self {
        SpaceMember {
            did: signer.did().to_owned(),
            signer,
            verifier,
            policy: JoinPolicy::default(),
            protocols: BTreeMap::new(),
            instances: BTreeMap::new(),
            journal: Vec::new(),
            refusals: Vec::new(),
            refused_instances: BTreeSet::new(),
            unacked: BTreeMap::new(),
        }
    }

    /// A participant for a local DID actor: [`ActorSigner`] and a [`ResolverVerifier`].
    pub fn for_actor(actor: &LocalActor, resolver: Arc<dyn DidResolver>) -> Self {
        Self::new(
            Arc::new(ActorSigner(actor.clone())),
            Arc::new(ResolverVerifier(resolver)),
        )
    }

    /// The same participant under `policy`.
    #[must_use]
    pub fn with_policy(mut self, policy: JoinPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Replaces the join policy.
    pub fn set_policy(&mut self, policy: JoinPolicy) {
        self.policy = policy;
    }

    /// Holds the protocol document `protocol`, so a genesis naming it can be read.
    pub fn know(&mut self, protocol: Arc<Protocol>) {
        self.protocols
            .insert(protocol.id().as_str().to_owned(), protocol);
    }

    /// The participant's DID.
    pub fn did(&self) -> &str {
        &self.did
    }

    /// The instance `id`, if the participant takes part in it.
    pub fn instance(&self, id: &str) -> Option<&Instance> {
        self.instances.get(id)
    }

    /// The journal, in append order.
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Every recorded refusal, in order, with the message hash when known.
    pub fn refusals(&self) -> &[(Option<MessageHash>, Refusal)] {
        &self.refusals
    }

    fn append(&mut self, instance: &str, message: Message) {
        self.journal.push(JournalEntry {
            instance: instance.to_owned(),
            message,
        });
    }

    fn refuse(&mut self, hash: Option<MessageHash>, refusal: Refusal) -> Outcome {
        self.refusals.push((hash.clone(), refusal.clone()));
        Outcome::Refused { hash, refusal }
    }

    /// Opens an instance as its initiator: signs the genesis, starts the instance and journals
    /// the genesis. The participant's own decision needs no policy. `parent` opens a child
    /// instance under a message of a parent instance it takes part in: `(parent id, message
    /// hash)`.
    ///
    /// # Errors
    ///
    /// [`SendError`] when signing fails or the genesis would be malformed.
    pub fn open(
        &mut self,
        protocol: Arc<Protocol>,
        role: &str,
        bindings: &BTreeMap<String, String>,
        parent: Option<(&str, &str)>,
    ) -> Result<Message, SendError> {
        self.know(protocol.clone());
        let parent_instance = match parent {
            Some((id, hash)) => Some((
                self.instances.get(id).ok_or_else(|| {
                    SendError::Refused(Reason::BadGenesis(format!("no parent instance {id}")))
                })?,
                hash,
            )),
            None => None,
        };
        let (genesis, inst) = Instance::create(
            protocol,
            self.signer.as_ref(),
            role,
            bindings,
            parent_instance,
        )?;
        let id = inst.id().to_owned();
        self.instances.insert(id.clone(), inst);
        self.append(&id, genesis.clone());
        Ok(genesis)
    }

    fn with_instance(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut Instance, &dyn Signer) -> Result<Message, SendError>,
    ) -> Result<Message, SendError> {
        let signer = self.signer.clone();
        let inst = self
            .instances
            .get_mut(id)
            .ok_or(SendError::Refused(Reason::WrongInstance))?;
        let msg = f(inst, signer.as_ref())?;
        self.append(id, msg.clone());
        Ok(msg)
    }

    /// Joins the instance `id` with a signed `join`, and journals it.
    ///
    /// # Errors
    ///
    /// As [`Instance::join`].
    pub fn join(&mut self, id: &str) -> Result<Message, SendError> {
        self.with_instance(id, |i, s| i.join(s))
    }

    /// Declines the instance `id` with a signed `decline`, and journals it.
    ///
    /// # Errors
    ///
    /// As [`Instance::decline`].
    pub fn decline(&mut self, id: &str) -> Result<Message, SendError> {
        self.with_instance(id, |i, s| i.decline(s))
    }

    /// Sends a message of the participant's role in instance `id`: the same check a receiver
    /// runs, on the participant's own projection; journals it when it may be sent.
    ///
    /// # Errors
    ///
    /// As [`Instance::send`].
    pub fn send(
        &mut self,
        id: &str,
        kind: &str,
        payload: Json,
        recipients: &[&str],
    ) -> Result<Message, SendError> {
        self.with_instance(id, |i, s| i.send(s, kind, payload, recipients))
    }

    /// The role-to-DID bindings `msg` is fanned out under: its instance's, or for a genesis the
    /// bindings it declares.
    pub fn bindings_for(&self, msg: &Message) -> BTreeMap<String, String> {
        let id = msg.body.instance.clone().unwrap_or_else(|| msg.hash());
        if let Some(i) = self.instances.get(&id) {
            return i.bindings().clone();
        }
        msg.body
            .payload
            .get("bindings")
            .and_then(Json::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(r, d)| d.as_str().map(|d| (r.clone(), d.to_owned())))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Takes in one protocol message, from any transport. A genesis goes through the join
    /// policy first, then the signature and the genesis checks; anything else through its
    /// instance's conformance check and causal delivery. Only what passes is journaled.
    // @tg: implements:: [[openspec:protocol-spaces#Join Policy]]
    pub fn receive(&mut self, msg: &Message, now_ms: u64) -> Vec<Outcome> {
        if msg.body.kind == control::GENESIS && msg.body.instance.is_none() {
            return vec![self.receive_genesis(msg)];
        }
        let hash = msg.hash();
        let Some(id) = msg.body.instance.clone() else {
            return vec![self.refuse(Some(hash), Refusal::Check(Reason::WrongInstance))];
        };
        if self.refused_instances.contains(&id) {
            return vec![self.refuse(Some(hash), Refusal::RefusedInstance(id))];
        }
        let verifier = self.verifier.clone();
        let Some(inst) = self.instances.get_mut(&id) else {
            return vec![Outcome::Deferred(hash)];
        };
        let events = inst.receive(msg, verifier.as_ref(), now_ms);
        self.absorb(&id, events)
    }

    fn receive_genesis(&mut self, genesis: &Message) -> Outcome {
        let hash = genesis.hash();
        if self.instances.contains_key(&hash) {
            return Outcome::Duplicate(hash);
        }
        // The policy reads the genesis alone: a refused genesis costs no signature check, no
        // resolution and no journal append.
        if let Err(r) = self.policy.decide(genesis, &self.did) {
            self.refused_instances.insert(hash.clone());
            return self.refuse(Some(hash), Refusal::Policy(r));
        }
        let Some(protocol) = self.protocols.get(&genesis.body.protocol).cloned() else {
            self.refused_instances.insert(hash.clone());
            return self.refuse(
                Some(hash),
                Refusal::UnknownProtocol(genesis.body.protocol.clone()),
            );
        };
        let parent = genesis
            .body
            .payload
            .get("parent")
            .and_then(Json::as_str)
            .and_then(|p| self.instances.values().find(|i| i.has_applied(p)));
        match Instance::from_genesis(protocol, genesis, &self.did, self.verifier.as_ref(), parent) {
            Ok(inst) => {
                self.instances.insert(hash.clone(), inst);
                self.append(&hash, genesis.clone());
                Outcome::Journaled {
                    instance: hash.clone(),
                    hash,
                }
            }
            Err(reason) => {
                self.refused_instances.insert(hash.clone());
                self.refuse(Some(hash), Refusal::Check(reason))
            }
        }
    }

    fn absorb(&mut self, id: &str, events: Vec<Event>) -> Vec<Outcome> {
        let mut out = Vec::new();
        for ev in events {
            out.push(match ev {
                Event::Applied(h) => {
                    if let Some(m) = self.instances.get(id).and_then(|i| i.message(&h)).cloned() {
                        self.append(id, m);
                    }
                    Outcome::Journaled {
                        instance: id.to_owned(),
                        hash: h,
                    }
                }
                Event::Duplicate(h) => Outcome::Duplicate(h),
                Event::Held(h) => Outcome::Held(h),
                Event::Refused { hash, reason, .. } => {
                    self.refuse(Some(hash), Refusal::Check(reason))
                }
                Event::Stalled { hash, missing } => Outcome::Stalled { hash, missing },
                Event::Equivocation(e) => Outcome::Equivocation(e),
            });
        }
        out
    }

    /// Records the hash of a message of instance `id` sealed to other roles, seen in the
    /// dependencies of a message this participant holds (see [`Instance::observe_hidden`]),
    /// and journals what it releases.
    pub fn observe_hidden(&mut self, id: &str, hash: &str) -> Vec<Outcome> {
        let verifier = self.verifier.clone();
        let Some(inst) = self.instances.get_mut(id) else {
            return Vec::new();
        };
        let events = inst.observe_hidden(hash, verifier.as_ref());
        self.absorb(id, events)
    }

    /// Drops held messages past their protocol's timeout in every instance.
    pub fn expire(&mut self, now_ms: u64) -> Vec<Outcome> {
        let ids: Vec<String> = self.instances.keys().cloned().collect();
        let mut out = Vec::new();
        for id in ids {
            if let Some(inst) = self.instances.get_mut(&id) {
                let events = inst.expire(now_ms);
                out.extend(self.absorb(&id, events));
            }
        }
        out
    }

    /// Picks up this participant's copies from a relay mailbox, opens each with `protocol` as
    /// `actor`, takes the message in and acknowledges a copy once it is journaled, a duplicate,
    /// or refused and recorded. A held or deferred copy stays at the relay until it settles.
    // @tg: implements:: [[openspec:protocol-spaces#Pairwise Sealed Fan-Out]]
    pub fn pickup(
        &mut self,
        actor: &LocalActor,
        source: &dyn PickupSource,
        protocol: &dyn RelayProtocol,
        resolver: &dyn DidResolver,
        now: Timestamp,
    ) -> Vec<Outcome> {
        let Ok(stored) = source.fetch(std::slice::from_ref(&actor.identity.did), 256) else {
            return Vec::new();
        };
        let now_ms = u64::try_from(now.0).unwrap_or(0);
        let mut out = Vec::new();
        let mut ack = Vec::new();
        for copy in stored {
            let msg = protocol
                .open(actor, &copy.bytes, resolver, now)
                .map_err(|e| e.to_string())
                .and_then(|opened| message_of(&opened.envelope));
            let outcomes = match msg {
                Ok(m) => {
                    let ids = self.unacked.entry(m.hash()).or_default();
                    if !ids.contains(&copy.id) {
                        ids.push(copy.id.clone());
                    }
                    self.receive(&m, now_ms)
                }
                Err(e) => {
                    ack.push(copy.id.clone());
                    vec![self.refuse(None, Refusal::Transport(e))]
                }
            };
            for o in &outcomes {
                if o.settles()
                    && let Some(h) = o.hash()
                    && let Some(ids) = self.unacked.remove(h)
                {
                    ack.extend(ids);
                }
            }
            out.extend(outcomes);
        }
        ack.sort();
        ack.dedup();
        if !ack.is_empty() {
            // A failed acknowledgement only means the copies come again, and are duplicates then.
            let _ = source.ack(&ack);
        }
        out
    }

    /// Takes in this participant's copies from an in-process channel. A deferred copy goes
    /// back on the channel; a held one waits in its instance.
    pub fn pickup_local(&mut self, channel: &LocalChannel, now_ms: u64) -> Vec<Outcome> {
        let mut out = Vec::new();
        let mut keep = Vec::new();
        for bytes in channel.take(&self.did) {
            let parsed = std::str::from_utf8(&bytes)
                .map_err(|e| e.to_string())
                .and_then(|t| Message::from_json(t).map_err(|e| e.to_string()));
            match parsed {
                Ok(m) => {
                    let outcomes = self.receive(&m, now_ms);
                    if outcomes.iter().any(|o| matches!(o, Outcome::Deferred(_))) {
                        keep.push(bytes);
                    }
                    out.extend(outcomes);
                }
                Err(e) => out.push(self.refuse(None, Refusal::Transport(e))),
            }
        }
        channel.requeue(&self.did, keep);
        out
    }
}
