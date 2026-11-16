//! Chart role binding: a DID actor plays agent protocols (Kolyada, `vertep-kolyada`) in the roles
//! its chart declares with `vt:plays`.
//!
//! A [`RoleBinding`] holds the protocols an actor plays and the instances it takes part in. The
//! instances are *derived* state: live, they advance as messages are admitted and sent; after a
//! restart they are rebuilt from the actor's journal ([`RoleBinding::recover`]) without
//! resolving a DID or verifying a signature, because the verdict was journaled with the input.
//!
//! - **Inbound** ([`RoleBinding::admit`]) runs at the grants slot of the relay inbox, before any
//!   journal append. A relay envelope named [`PROTOCOL_EVENT`] carries one signed message as JSON
//!   text; the binding verifies it, runs the conformance check of its instance and returns the
//!   events to journal, or the [`Refusal`] to record. Grants are not consulted: the role is the
//!   authority, and no grant widens it.
//! - **The chart** sees an ordinary event named by the message type (control messages as
//!   `kolyada.genesis`, `kolyada.join`, `kolyada.decline`, `kolyada.complete` and
//!   `kolyada.violation`), with the verified signer as `_event.origin` and data
//!   `{protocol, instance, role, verdict, payload, message}`: guards read `_event.data.instance`
//!   and `_event.data.role`, the payload is `_event.data.payload`.
//! - **Outbound** ([`RoleBinding::outbound`]) runs in the send path of the relay outbox. A send
//!   whose data names one of the actor's instances (`{instance, payload, recipients?}`) is
//!   checked against the actor's own projection and signed; a send the role may not make fails
//!   locally with `error.communication` and nothing leaves. The signed message is fed back to
//!   the actor as a [`SENT_EVENT`] input, so its journal holds every message its instances
//!   applied.
//! - **Discovery**: the `VertepProtocol` service entry is generated from the declarations
//!   ([`RoleBinding::service_entry`]).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{Value as Json, json};
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_core::ir::Chart;
use vertep_core::plays::{PlaysError, RolePlay, plays};
use vertep_kolyada::envelope::{SignError, Signer, Verifier, VerifyError, control};
use vertep_kolyada::offer::{ServiceProtocol, service_entry};
use vertep_kolyada::{Event as KEvent, Instance, Message, MessageHash, Protocol, Reason};
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{Address, EntryData, Envelope, Storage};

use crate::did::Did;
use crate::envelope::JsonValue;
use crate::error::{CommunicationError, CommunicationFailure};
use crate::origin::{Admitted, OpenedMessage, Refusal, VerifiedOrigin};
use crate::relay::LocalActor;
use crate::resolver::DidResolver;
use crate::time::Timestamp;

/// The relay event that carries one signed protocol message, as JSON text in its data.
pub const PROTOCOL_EVENT: &str = "kolyada.message";

/// The input that records a message the actor itself signed and sent.
pub const SENT_EVENT: &str = "kolyada.sent";

/// The verdict journaled with every protocol input.
pub const ACCEPTED: &str = "accepted";

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The event name the chart sees for a message kind.
#[must_use]
pub fn event_name(kind: &str) -> String {
    match kind.strip_prefix("kolyada:") {
        Some(control) => format!("kolyada.{control}"),
        None => kind.to_owned(),
    }
}

fn kind_of(name: &str) -> String {
    match name.strip_prefix("kolyada.") {
        Some(c) if c != "message" && c != "sent" => format!("kolyada:{c}"),
        _ => name.to_owned(),
    }
}

fn to_json(v: &Value) -> Json {
    serde_json::to_value(JsonValue(v.clone())).unwrap_or(Json::Null)
}

fn from_json(v: &Json) -> Value {
    serde_json::from_value::<JsonValue>(v.clone())
        .map(|j| j.0)
        .unwrap_or(Value::Null)
}

fn member<'v>(v: &'v Value, key: &str) -> Option<&'v Value> {
    match v {
        Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn text(v: Option<&Value>) -> Option<&str> {
    match v {
        Some(Value::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Signs protocol messages as a local DID actor, through its key manager.
pub struct KeySigner {
    actor: LocalActor,
}

impl KeySigner {
    /// The signer of `actor`, with its authentication key.
    #[must_use]
    pub fn new(actor: LocalActor) -> Self {
        KeySigner { actor }
    }
}

impl Signer for KeySigner {
    fn did(&self) -> &str {
        &self.actor.identity.did
    }

    fn key_id(&self) -> &str {
        &self.actor.identity.signing.kid
    }

    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, SignError> {
        self.actor
            .km
            .sign(&self.actor.identity.signing.key_name, data)
            .map_err(|e| SignError(e.to_string()))
    }
}

/// Verifies protocol signatures against the signer's resolved DID document: the key must be
/// one of its authentication or assertion methods.
pub struct ResolverVerifier<'r> {
    resolver: &'r dyn DidResolver,
}

impl<'r> ResolverVerifier<'r> {
    /// A verifier over `resolver`.
    #[must_use]
    pub fn new(resolver: &'r dyn DidResolver) -> Self {
        ResolverVerifier { resolver }
    }
}

impl Verifier for ResolverVerifier<'_> {
    fn verify(
        &self,
        did: &str,
        key_id: &str,
        data: &[u8],
        signature: &[u8],
    ) -> Result<(), VerifyError> {
        let parsed = Did::parse(did).map_err(|e| VerifyError(e.to_string()))?;
        let doc = self
            .resolver
            .resolve(&parsed)
            .map_err(|e| VerifyError(e.to_string()))?
            .document;
        let key = doc
            .authentication_methods()
            .into_iter()
            .chain(doc.assertion_methods())
            .find(|m| m.id == key_id)
            .ok_or_else(|| VerifyError(format!("{key_id} is not a signing key of {did}")))?
            .public_key;
        if key.verify(data, signature) {
            Ok(())
        } else {
            Err(VerifyError("signature mismatch".into()))
        }
    }
}

/// Replay trusts the journal: the verdict was journaled with the input, so nothing is
/// verified again.
struct Journaled;

impl Verifier for Journaled {
    fn verify(&self, _: &str, _: &str, _: &[u8], _: &[u8]) -> Result<(), VerifyError> {
        Ok(())
    }
}

/// Decides whether the actor takes part in an instance a genesis opens: the agent's join
/// policy, run before the genesis is journaled.
pub type GenesisPolicy = Arc<dyn Fn(&Message, &Instance) -> bool + Send + Sync>;

/// A chart's role declarations do not fit the protocols given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingError {
    /// A malformed `vt:plays` token.
    Plays(PlaysError),
    /// The chart plays a protocol whose document was not given.
    MissingProtocol(String),
    /// The chart plays a role the protocol does not declare.
    UnknownRole {
        /// The protocol identity.
        protocol: String,
        /// The role.
        role: String,
    },
    /// The journal could not be read.
    Journal(String),
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::Plays(e) => e.fmt(f),
            BindingError::MissingProtocol(p) => write!(f, "no document for protocol {p}"),
            BindingError::UnknownRole { protocol, role } => {
                write!(f, "protocol {protocol} declares no role {role}")
            }
            BindingError::Journal(m) => write!(f, "journal: {m}"),
        }
    }
}

impl std::error::Error for BindingError {}

#[derive(Default)]
struct BindingState {
    instances: BTreeMap<String, Instance>,
    /// Messages held for their dependencies, by hash, so they can be delivered when applied.
    held: BTreeMap<MessageHash, Message>,
    /// Messages refused while held (evicted from a full buffer, or invalid once unblocked).
    refused: Vec<(MessageHash, Reason)>,
}

/// The protocol roles a DID actor plays and the instances it takes part in.
pub struct RoleBinding {
    did: String,
    plays: Vec<RolePlay>,
    protocols: BTreeMap<String, Arc<Protocol>>,
    policy: GenesisPolicy,
    state: Mutex<BindingState>,
}

impl fmt::Debug for RoleBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoleBinding")
            .field("did", &self.did)
            .field("plays", &self.plays)
            .finish_non_exhaustive()
    }
}

/// The verdict of the outbound check: the envelope that leaves and the input that records it.
#[derive(Clone, Debug)]
pub struct Outbound {
    /// The relay envelope carrying the signed message to the send's target.
    pub envelope: Envelope,
    /// The [`SENT_EVENT`] input for the sending actor's own journal.
    pub sent: Envelope,
    /// The signed message.
    pub message: Message,
}

impl RoleBinding {
    /// The binding of the actor `did` whose chart is `chart`, with the documents of the
    /// protocols it plays. Every protocol `vt:plays` names must be given and declare the role.
    /// The join policy refuses every genesis until [`RoleBinding::with_genesis_policy`] sets one.
    ///
    /// # Errors
    ///
    /// [`BindingError`] for a malformed declaration, a missing document or an unknown role.
    // @tg: implements:: [[openspec:protocol-spaces#Chart Role Binding]]
    pub fn new(
        did: impl Into<String>,
        chart: &Chart,
        protocols: impl IntoIterator<Item = Arc<Protocol>>,
    ) -> Result<Self, BindingError> {
        let declared = plays(chart).map_err(BindingError::Plays)?;
        Self::from_plays(did, declared, protocols)
    }

    /// The binding for explicit role declarations, as [`RoleBinding::new`] reads them.
    ///
    /// # Errors
    ///
    /// [`BindingError`] for a missing document or an unknown role.
    pub fn from_plays(
        did: impl Into<String>,
        declared: Vec<RolePlay>,
        protocols: impl IntoIterator<Item = Arc<Protocol>>,
    ) -> Result<Self, BindingError> {
        let given: BTreeMap<String, Arc<Protocol>> = protocols
            .into_iter()
            .map(|p| (p.id().as_str().to_owned(), p))
            .collect();
        let mut used = BTreeMap::new();
        for p in &declared {
            let doc = given
                .get(&p.protocol)
                .ok_or_else(|| BindingError::MissingProtocol(p.protocol.clone()))?;
            if !doc.has_role(&p.role) {
                return Err(BindingError::UnknownRole {
                    protocol: p.protocol.clone(),
                    role: p.role.clone(),
                });
            }
            used.insert(p.protocol.clone(), doc.clone());
        }
        Ok(RoleBinding {
            did: did.into(),
            plays: declared,
            protocols: used,
            policy: Arc::new(|_, _| false),
            state: Mutex::new(BindingState::default()),
        })
    }

    /// Sets the join policy run on every genesis before it is journaled.
    #[must_use]
    pub fn with_genesis_policy(mut self, policy: GenesisPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// The actor's DID.
    #[must_use]
    pub fn did(&self) -> &str {
        &self.did
    }

    /// The roles declared, sorted.
    #[must_use]
    pub fn plays(&self) -> &[RolePlay] {
        &self.plays
    }

    fn plays_role(&self, protocol: &str, role: &str) -> bool {
        self.plays
            .iter()
            .any(|p| p.protocol == protocol && p.role == role)
    }

    /// The protocols with the roles played, one entry per protocol.
    #[must_use]
    pub fn service_protocols(&self) -> Vec<ServiceProtocol> {
        let mut by: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for p in &self.plays {
            by.entry(&p.protocol).or_default().insert(&p.role);
        }
        by.into_iter()
            .map(|(protocol, roles)| ServiceProtocol {
                protocol: protocol.to_owned(),
                roles: roles.into_iter().map(str::to_owned).collect(),
            })
            .collect()
    }

    /// The `VertepProtocol` service entry of the actor's DID document, generated from the
    /// chart's `vt:plays` declarations.
    // @tg: implements:: [[openspec:agent-protocols#Protocol Discovery]] {note: "generated from vt:plays"}
    #[must_use]
    pub fn service_entry(&self) -> Json {
        service_entry(&self.did, &self.service_protocols())
    }

    /// Puts the generated service entry into a DID document in JSON, replacing any
    /// `VertepProtocol` entry it had.
    pub fn publish_into(&self, did_document: &mut Json) {
        let Some(doc) = did_document.as_object_mut() else {
            return;
        };
        let services = doc.entry("service").or_insert_with(|| json!([]));
        if let Some(list) = services.as_array_mut() {
            list.retain(|s| {
                s.get("type").and_then(Json::as_str) != Some(vertep_kolyada::offer::SERVICE_TYPE)
            });
            list.push(self.service_entry());
        }
    }

    /// The instance `id`, as this actor sees it.
    #[must_use]
    pub fn instance(&self, id: &str) -> Option<Instance> {
        lock(&self.state).instances.get(id).cloned()
    }

    /// The ids of the instances this actor takes part in.
    #[must_use]
    pub fn instances(&self) -> Vec<String> {
        lock(&self.state).instances.keys().cloned().collect()
    }

    /// Messages refused while they waited for dependencies, with the reasons.
    #[must_use]
    pub fn refused_while_held(&self) -> Vec<(MessageHash, Reason)> {
        lock(&self.state).refused.clone()
    }

    fn input(&self, msg: &Message, name: &str) -> Event {
        let b = &msg.body;
        let instance = b.instance.clone().unwrap_or_else(|| msg.hash());
        let mut e = Event::named(name, EventType::External);
        e.data = Value::Object(vec![
            ("protocol".into(), Value::String(b.protocol.clone())),
            ("instance".into(), Value::String(instance)),
            ("role".into(), Value::String(b.role.clone())),
            ("verdict".into(), Value::String(ACCEPTED.into())),
            ("payload".into(), from_json(&b.payload)),
            ("message".into(), Value::String(msg.to_json())),
        ]);
        e
    }

    fn admitted(&self, msg: &Message, io_processor: &str) -> Admitted {
        let origin = VerifiedOrigin::verified(&msg.body.sender, &msg.key, io_processor);
        let mut event = self.input(msg, &event_name(&msg.body.kind));
        origin.apply(&mut event);
        Admitted {
            message_id: format!("kolyada:{}", msg.hash()),
            event,
            origin,
        }
    }

    /// The inbound check, run at the grants slot before any journal append: verifies the signed
    /// message `opened` carries, judges it in its instance and returns the events to journal, in
    /// the order they apply. A message that waits for its dependencies, or that was already
    /// applied, gives no event; when a message arrives that unblocks held ones, they follow it.
    ///
    /// # Errors
    ///
    /// The [`Refusal`] to record: nothing is journaled.
    // @tg: implements:: [[openspec:protocol-spaces#Chart Role Binding]]
    // @tg: implements:: [[openspec:protocol-spaces#Role-Derived Authority]] {note: "no grant is consulted for protocol messages"}
    pub fn admit(
        &self,
        opened: &OpenedMessage,
        verifier: &dyn Verifier,
        now: Timestamp,
    ) -> Result<Vec<Admitted>, Refusal> {
        let text = match &opened.envelope.data.0 {
            Value::String(t) => t.clone(),
            _ => {
                return Err(Refusal::MalformedEnvelope(
                    "a protocol message is JSON text".into(),
                ));
            }
        };
        let msg =
            Message::from_json(&text).map_err(|e| Refusal::MalformedEnvelope(e.to_string()))?;
        let io = opened.origin.io_processor.clone();
        let Some(protocol) = self.protocols.get(&msg.body.protocol).cloned() else {
            return Err(Refusal::Protocol(format!(
                "{} is not a protocol {} plays",
                msg.body.protocol, self.did
            )));
        };
        let mut st = lock(&self.state);
        if msg.body.kind == control::GENESIS {
            let id = msg.hash();
            if st.instances.contains_key(&id) {
                return Ok(Vec::new());
            }
            let inst = Instance::from_genesis(protocol, &msg, &self.did, verifier, None)
                .map_err(|r| Refusal::Protocol(r.to_string()))?;
            if !self.plays_role(&msg.body.protocol, inst.local_role()) {
                return Err(Refusal::Protocol(format!(
                    "{} does not play {}",
                    self.did,
                    inst.local_role()
                )));
            }
            if !(self.policy)(&msg, &inst) {
                return Err(Refusal::Protocol(
                    "the join policy refused the genesis".into(),
                ));
            }
            st.instances.insert(id, inst);
            return Ok(vec![self.admitted(&msg, &io)]);
        }
        let Some(id) = msg.body.instance.clone() else {
            return Err(Refusal::Protocol("the message names no instance".into()));
        };
        let BindingState {
            instances,
            held,
            refused,
        } = &mut *st;
        let Some(inst) = instances.get_mut(&id) else {
            return Err(Refusal::Protocol(format!("unknown instance {id}")));
        };
        let trigger = msg.hash();
        let now_ms = u64::try_from(now.millis()).unwrap_or(0);
        let mut out = Vec::new();
        for ev in inst.receive(&msg, verifier, now_ms) {
            match ev {
                KEvent::Applied(h) => {
                    let m = if h == trigger {
                        Some(msg.clone())
                    } else {
                        held.remove(&h)
                    };
                    if let Some(m) = m {
                        out.push(self.admitted(&m, &io));
                    }
                }
                KEvent::Held(h) => {
                    held.insert(h, msg.clone());
                }
                KEvent::Refused { hash, reason, .. } if hash == trigger => {
                    return Err(Refusal::Protocol(reason.to_string()));
                }
                KEvent::Refused { hash, reason, .. } => {
                    held.remove(&hash);
                    refused.push((hash, reason));
                }
                KEvent::Duplicate(_) | KEvent::Stalled { .. } | KEvent::Equivocation(_) => {}
            }
        }
        Ok(out)
    }

    /// True when `event`, sent by this actor, is a protocol send: its data names one of the
    /// actor's instances, or it is a `kolyada.genesis` for a protocol the actor plays.
    #[must_use]
    pub fn is_protocol_send(&self, event: &Event) -> bool {
        if event.name == event_name(control::GENESIS) {
            return text(member(&event.data, "protocol"))
                .is_some_and(|p| self.protocols.contains_key(p));
        }
        text(member(&event.data, "instance"))
            .is_some_and(|id| lock(&self.state).instances.contains_key(id))
    }

    /// The outbound check, run in the send path before anything is sealed: builds the message
    /// `env` asks for, checks it against the actor's own projection, signs it and applies it.
    ///
    /// - `kolyada.genesis` with data `{protocol, role, bindings}` opens an instance;
    /// - `kolyada.join` and `kolyada.decline` with data `{instance}` answer one;
    /// - any other name is a message type, with data `{instance, payload, recipients?}`; the
    ///   recipients default to the roles bound to the send's target.
    ///
    /// # Errors
    ///
    /// `error.communication` with the protocol's reason when the role may not send it; nothing
    /// is signed or applied.
    // @tg: implements:: [[openspec:protocol-spaces#Chart Role Binding]]
    pub fn outbound(
        &self,
        signer: &dyn Signer,
        env: &Envelope,
    ) -> Result<Outbound, CommunicationError> {
        let refused = |m: String| {
            let e = CommunicationError::new(CommunicationFailure::ProtocolRefused(m));
            match &env.event.sendid {
                Some(id) => e.for_send(id.clone()),
                None => e,
            }
        };
        let data = &env.event.data;
        let kind = kind_of(&env.event.name);
        let mut st = lock(&self.state);
        let msg = if kind == control::GENESIS {
            let protocol_id = text(member(data, "protocol")).unwrap_or_default();
            let role = text(member(data, "role")).unwrap_or_default();
            let protocol = self.protocols.get(protocol_id).cloned().ok_or_else(|| {
                refused(format!("{protocol_id} is not a protocol this actor plays"))
            })?;
            if !self.plays_role(protocol_id, role) {
                return Err(refused(format!("this actor does not play {role}")));
            }
            let bindings: BTreeMap<String, String> = match member(data, "bindings") {
                Some(Value::Object(m)) => m
                    .iter()
                    .filter_map(|(k, v)| match v {
                        Value::String(d) => Some((k.clone(), d.clone())),
                        _ => None,
                    })
                    .collect(),
                _ => BTreeMap::new(),
            };
            let (msg, inst) = Instance::create(protocol, signer, role, &bindings, None)
                .map_err(|e| refused(e.to_string()))?;
            st.instances.insert(msg.hash(), inst);
            msg
        } else {
            let id = text(member(data, "instance"))
                .unwrap_or_default()
                .to_owned();
            let inst = st
                .instances
                .get_mut(&id)
                .ok_or_else(|| refused(format!("unknown instance {id}")))?;
            match kind.as_str() {
                control::JOIN => inst.join(signer),
                control::DECLINE => inst.decline(signer),
                _ => {
                    let recipients: Vec<String> = match member(data, "recipients") {
                        Some(Value::Array(a)) => a
                            .iter()
                            .filter_map(|v| match v {
                                Value::String(s) => Some(s.clone()),
                                _ => None,
                            })
                            .collect(),
                        _ => inst
                            .bindings()
                            .iter()
                            .filter(|(_, d)| **d == env.to)
                            .map(|(r, _)| r.clone())
                            .collect(),
                    };
                    let refs: Vec<&str> = recipients.iter().map(String::as_str).collect();
                    let payload = member(data, "payload").map_or(Json::Null, to_json);
                    let payload = if payload.is_null() {
                        json!({})
                    } else {
                        payload
                    };
                    inst.send(signer, &kind, payload, &refs)
                }
            }
            .map_err(|e| refused(e.to_string()))?
        };
        drop(st);
        let mut wire = env.clone();
        wire.event.name = PROTOCOL_EVENT.to_owned();
        wire.event.data = Value::String(msg.to_json());
        let me = Address::parse(&self.did).map_err(|e| refused(e.to_string()))?;
        let mut input = self.input(&msg, SENT_EVENT);
        input.origin = Some(self.did.clone());
        let sent = Envelope::new(format!("kolyada:{}", msg.hash()), &me, input);
        Ok(Outbound {
            envelope: wire,
            sent,
            message: msg,
        })
    }

    /// Rebuilds the instances from journaled protocol inputs, in journal order, without
    /// resolving any DID or verifying any signature. Returns how many inputs were applied.
    ///
    /// # Errors
    ///
    /// [`BindingError::Journal`] for a journaled protocol input that does not apply.
    // @tg: implements:: [[openspec:protocol-spaces#Chart Role Binding]] {note: "checker state derived on replay"}
    pub fn replay<'e>(
        &self,
        events: impl IntoIterator<Item = &'e Event>,
    ) -> Result<usize, BindingError> {
        let mut st = lock(&self.state);
        let mut n = 0;
        for e in events {
            if text(member(&e.data, "verdict")) != Some(ACCEPTED) {
                continue;
            }
            let Some(m) = text(member(&e.data, "message")) else {
                continue;
            };
            let Ok(msg) = Message::from_json(m) else {
                continue;
            };
            if e.name != SENT_EVENT && e.name != event_name(&msg.body.kind) {
                continue;
            }
            let Some(protocol) = self.protocols.get(&msg.body.protocol).cloned() else {
                continue;
            };
            if msg.body.kind == control::GENESIS {
                if let std::collections::btree_map::Entry::Vacant(slot) =
                    st.instances.entry(msg.hash())
                {
                    let inst = Instance::from_genesis(protocol, &msg, &self.did, &Journaled, None)
                        .map_err(|r| {
                            BindingError::Journal(format!("genesis {}: {r}", msg.hash()))
                        })?;
                    slot.insert(inst);
                }
                n += 1;
                continue;
            }
            let id = msg.body.instance.clone().unwrap_or_default();
            let inst = st
                .instances
                .get_mut(&id)
                .ok_or_else(|| BindingError::Journal(format!("unknown instance {id}")))?;
            for ev in inst.receive(&msg, &Journaled, 0) {
                if let KEvent::Refused { reason, .. } = ev {
                    return Err(BindingError::Journal(format!(
                        "{} in {id}: {reason}",
                        msg.hash()
                    )));
                }
            }
            n += 1;
        }
        Ok(n)
    }

    /// Rebuilds the instances from the journal of `actor` in `storage`.
    ///
    /// # Errors
    ///
    /// [`BindingError::Journal`] when the journal cannot be read or does not apply.
    pub fn recover(&self, storage: &dyn Storage, actor: &Address) -> Result<usize, BindingError> {
        let records = storage
            .journal()
            .read(actor.node(), vertep_runtime::vertep_store::Position::ZERO)
            .map_err(|e| BindingError::Journal(e.to_string()))?;
        let mut events = Vec::new();
        for r in &records {
            let entry = EntryData::from_record(actor, r)
                .map_err(|e| BindingError::Journal(e.to_string()))?;
            if let EntryBody::Input { input, .. } = entry.body {
                events.push(input.event);
            }
        }
        self.replay(events.iter())
    }
}
