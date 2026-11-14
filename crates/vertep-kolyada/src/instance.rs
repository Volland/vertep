//! A protocol instance as one participant sees it: signed genesis, countersigned joins,
//! role-derived authority, causal delivery and a hash-linked history.
//!
//! [`Instance::check`] is the conformance function: it reads the instance and a message and
//! returns a [`Verdict`] without changing anything. [`Instance::receive`] adds causal delivery
//! around it: a message whose dependencies have not arrived is held, in a buffer the protocol
//! sizes, until they do or its timeout passes.
//!
//! A participant knows the protocol state only through the messages it sees. Its knowledge is
//! the set of states consistent with them: a transition it takes no part in is silent and adds
//! its target, and a message hidden from it counts as one unknown step. A message is enabled
//! when some state in the set enables it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::document::{Protocol, Transition};
use crate::envelope::{Body, Message, MessageHash, SignError, Signer, Verifier, control};
use crate::violation::Equivocation;

/// Where an instance is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The genesis is in, and not every bound role has joined.
    Pending,
    /// Every bound role has joined; protocol messages are checked against the projection.
    Open,
    /// A role declined; the instance never opens.
    Declined,
    /// The protocol reached a final state and a participant said so.
    Completed,
}

/// Why a message is refused. [`Reason::as_str`] gives the fixed text recorded with a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The signature does not verify against the sender's DID.
    Signature,
    /// The message names another protocol or instance.
    WrongInstance,
    /// The sender is not bound to the role it names, or the role may not send this message here.
    RoleNotPermitted,
    /// The instance is not open.
    NotOpen,
    /// A message it depends on has not been applied.
    MissingDependencies,
    /// No state the participant may be in enables the message.
    NotEnabledInState,
    /// The protocol declares no such message type.
    UnknownMessage,
    /// The payload does not match the declared shape; the text says where.
    PayloadShape(String),
    /// The recipient set is empty, names unknown roles or leaves out a required receiver.
    Recipients,
    /// The sequence number does not rise.
    Sequence,
    /// A second, different message with the same role and sequence number.
    Equivocation,
    /// The causal-delivery buffer is full.
    BufferFull,
    /// The genesis is malformed; the text says how.
    BadGenesis(String),
    /// The role has already joined.
    AlreadyJoined,
}

impl Reason {
    /// The fixed text of the reason.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Signature => "signature",
            Self::WrongInstance => "wrong instance",
            Self::RoleNotPermitted => "role not permitted",
            Self::NotOpen => "not open",
            Self::MissingDependencies => "missing dependencies",
            Self::NotEnabledInState => "not enabled in state",
            Self::UnknownMessage => "unknown message",
            Self::PayloadShape(_) => "payload shape",
            Self::Recipients => "recipients",
            Self::Sequence => "sequence",
            Self::Equivocation => "equivocation",
            Self::BufferFull => "buffer full",
            Self::BadGenesis(_) => "bad genesis",
            Self::AlreadyJoined => "already joined",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadShape(d) | Self::BadGenesis(d) => write!(f, "{}: {d}", self.as_str()),
            _ => f.write_str(self.as_str()),
        }
    }
}

/// The verdict of the conformance check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The message may be applied.
    Accepted,
    /// The message was already applied.
    Duplicate,
    /// The message is refused.
    Refused(Reason),
}

/// What [`Instance::receive`] did with a message, or with one it had been holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The message was applied.
    Applied(MessageHash),
    /// The message had already been applied and is dropped.
    Duplicate(MessageHash),
    /// The message waits for dependencies.
    Held(MessageHash),
    /// The message is refused; the signed message is kept for a [`crate::Violation`].
    Refused {
        /// The message hash.
        hash: MessageHash,
        /// Why.
        reason: Reason,
        /// The message.
        message: Box<Message>,
    },
    /// A held message waited past the protocol's timeout; the chart takes its cancel path.
    Stalled {
        /// The message hash.
        hash: MessageHash,
        /// The dependencies it still lacks.
        missing: Vec<MessageHash>,
    },
    /// Two different signed messages from one role with one sequence number.
    Equivocation(Box<Equivocation>),
}

/// An instance could not be created or rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceError(pub String);

impl fmt::Display for InstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InstanceError {}

/// A message could not be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The participant's own projection does not allow the message.
    Refused(Reason),
    /// The signer failed.
    Sign(SignError),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "send refused: {r}"),
            Self::Sign(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SendError {}

/// One entry of a forwarded history: the message if the receiver may read it, its hash if not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryEntry {
    /// A message the receiver is entitled to.
    Full(Box<Message>),
    /// A message hidden from the receiver.
    Hidden(MessageHash),
}

#[derive(Clone, Debug)]
struct Held {
    message: Message,
    arrived_ms: u64,
}

/// A protocol instance as one participant sees it.
#[derive(Clone, Debug)]
pub struct Instance {
    protocol: Arc<Protocol>,
    id: String,
    bindings: BTreeMap<String, String>,
    parent: Option<String>,
    local_role: String,
    status: Status,
    joined: BTreeSet<String>,
    applied: BTreeSet<MessageHash>,
    order: Vec<MessageHash>,
    messages: BTreeMap<MessageHash, Message>,
    heads: BTreeSet<MessageHash>,
    last_seq: BTreeMap<String, u64>,
    seen_seq: BTreeMap<(String, u64), MessageHash>,
    knowledge: BTreeSet<String>,
    held: Vec<Held>,
}

fn is_empty_payload(v: &Value) -> bool {
    v.is_null() || v.as_object().is_some_and(serde_json::Map::is_empty)
}

impl Instance {
    /// The instance id, the genesis hash.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The protocol.
    #[must_use]
    pub fn protocol(&self) -> &Protocol {
        &self.protocol
    }

    /// The role this participant plays.
    #[must_use]
    pub fn local_role(&self) -> &str {
        &self.local_role
    }

    /// The DID bound to each role.
    #[must_use]
    pub fn bindings(&self) -> &BTreeMap<String, String> {
        &self.bindings
    }

    /// The parent message that opened this instance, for a sub-protocol.
    #[must_use]
    pub fn parent(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    /// Where the instance is in its life.
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// The states this participant may be in, given what it has seen.
    #[must_use]
    pub fn knowledge(&self) -> &BTreeSet<String> {
        &self.knowledge
    }

    /// Whether every state this participant may be in is final.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.knowledge.is_empty()
            && self
                .knowledge
                .iter()
                .all(|s| self.protocol.finals().contains(s))
    }

    /// The hashes no applied message depends on; they name the history's frontier.
    #[must_use]
    pub fn heads(&self) -> Vec<MessageHash> {
        self.heads.iter().cloned().collect()
    }

    /// Whether the message with this hash has been applied.
    #[must_use]
    pub fn has_applied(&self, hash: &str) -> bool {
        self.applied.contains(hash)
    }

    /// The hashes in `other_heads` this participant has not applied: what to ask for.
    // @tg: implements:: [[openspec:protocol-spaces#Hash-Linked History]]
    #[must_use]
    pub fn missing_from(&self, other_heads: &[MessageHash]) -> Vec<MessageHash> {
        other_heads
            .iter()
            .filter(|h| !self.applied.contains(*h))
            .cloned()
            .collect()
    }

    /// How many messages wait for dependencies.
    #[must_use]
    pub fn held_len(&self) -> usize {
        self.held.len()
    }

    fn closure(&self, role: &str, mut set: BTreeSet<String>) -> BTreeSet<String> {
        loop {
            let before = set.len();
            for t in self.protocol.transitions() {
                if set.contains(&t.from) && t.sender != role && !t.receivers.contains(role) {
                    set.insert(t.to.clone());
                }
            }
            if set.len() == before {
                return set;
            }
        }
    }

    // @tg: implements:: [[openspec:protocol-spaces#Sub-Protocol Invocation]]
    fn init(
        protocol: Arc<Protocol>,
        genesis: &Message,
        local_did: &str,
        parent: Option<&Instance>,
    ) -> Result<Self, Reason> {
        let bad = |m: &str| Reason::BadGenesis(m.to_owned());
        let b = &genesis.body;
        if b.kind != control::GENESIS || b.instance.is_some() {
            return Err(bad("not a genesis"));
        }
        if b.protocol != protocol.id().as_str() {
            return Err(bad("names another protocol"));
        }
        let bindings: BTreeMap<String, String> = b
            .payload
            .get("bindings")
            .and_then(Value::as_object)
            .ok_or_else(|| bad("no bindings"))?
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|d| (k.clone(), d.to_owned()))
                    .ok_or_else(|| bad("binding is not a DID"))
            })
            .collect::<Result<_, _>>()?;
        let roles: BTreeSet<&String> = protocol.roles().iter().collect();
        if bindings.keys().collect::<BTreeSet<_>>() != roles {
            return Err(bad("bindings do not cover exactly the roles"));
        }
        if bindings.get(&b.role) != Some(&b.sender) {
            return Err(bad("the sender is not bound to its role"));
        }
        let others: BTreeSet<&String> = protocol.roles().iter().filter(|r| **r != b.role).collect();
        if b.recipients.iter().collect::<BTreeSet<_>>() != others {
            return Err(bad("recipients are not the other roles"));
        }
        let parent_hash = b
            .payload
            .get("parent")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(ph) = &parent_hash {
            let p = parent.ok_or_else(|| bad("names a parent that is not known"))?;
            if !p.applied.contains(ph) {
                return Err(bad("the parent message is not in the parent's history"));
            }
            let parent_dids: BTreeSet<&String> = p.bindings.values().collect();
            if bindings.values().any(|d| !parent_dids.contains(d)) {
                return Err(bad("a role is bound to a DID outside the parent"));
            }
        } else if b.payload.get("parent").is_some_and(|v| !v.is_null()) {
            return Err(bad("parent is not a hash"));
        }
        let local_role = bindings
            .iter()
            .find(|(_, d)| d.as_str() == local_did)
            .map(|(r, _)| r.clone())
            .ok_or_else(|| bad("the local DID is not bound"))?;
        let hash = genesis.hash();
        let mut inst = Self {
            id: hash.clone(),
            parent: parent_hash,
            bindings,
            local_role: local_role.clone(),
            status: Status::Pending,
            joined: BTreeSet::new(),
            applied: BTreeSet::from([hash.clone()]),
            order: vec![hash.clone()],
            messages: BTreeMap::from([(hash.clone(), genesis.clone())]),
            heads: BTreeSet::from([hash.clone()]),
            last_seq: BTreeMap::from([(b.role.clone(), b.seq)]),
            seen_seq: BTreeMap::from([((b.role.clone(), b.seq), hash)]),
            knowledge: BTreeSet::new(),
            held: Vec::new(),
            protocol,
        };
        inst.knowledge = inst.closure(
            &local_role,
            BTreeSet::from([inst.protocol.initial().to_owned()]),
        );
        let initiator_did = b.sender.clone();
        for (r, d) in &inst.bindings {
            if *d == initiator_did {
                inst.joined.insert(r.clone());
            }
        }
        inst.refresh_status();
        Ok(inst)
    }

    /// Reads a genesis and starts the instance for `local_did`, after verifying its signature.
    ///
    /// # Errors
    ///
    /// [`Reason::Signature`] for a bad signature, [`Reason::BadGenesis`] for a malformed genesis.
    // @tg: implements:: [[openspec:protocol-spaces#Protocol Instance]]
    pub fn from_genesis(
        protocol: Arc<Protocol>,
        genesis: &Message,
        local_did: &str,
        verifier: &dyn Verifier,
        parent: Option<&Instance>,
    ) -> Result<Self, Reason> {
        genesis.verify(verifier).map_err(|_| Reason::Signature)?;
        Self::init(protocol, genesis, local_did, parent)
    }

    /// Signs a genesis as the initiator and starts the instance. `bindings` gives a DID for
    /// every role, including the signer's own.
    ///
    /// # Errors
    ///
    /// [`SendError`] when signing fails or the genesis would be malformed.
    // @tg: implements:: [[openspec:protocol-spaces#Protocol Instance]]
    pub fn create(
        protocol: Arc<Protocol>,
        signer: &dyn Signer,
        role: &str,
        bindings: &BTreeMap<String, String>,
        parent: Option<(&Instance, &str)>,
    ) -> Result<(Message, Self), SendError> {
        let mut payload = json!({ "bindings": bindings });
        if let Some((_, parent_hash)) = parent {
            payload["parent"] = json!(parent_hash);
        }
        let body = Body {
            protocol: protocol.id().as_str().to_owned(),
            instance: None,
            sender: signer.did().to_owned(),
            role: role.to_owned(),
            seq: 1,
            deps: Vec::new(),
            recipients: protocol
                .roles()
                .iter()
                .filter(|r| *r != role)
                .cloned()
                .collect(),
            kind: control::GENESIS.to_owned(),
            payload,
        };
        let msg = Message::sign(body, signer).map_err(SendError::Sign)?;
        let inst = Self::init(protocol, &msg, signer.did(), parent.map(|p| p.0))
            .map_err(SendError::Refused)?;
        Ok((msg, inst))
    }

    fn refresh_status(&mut self) {
        if self.status == Status::Pending && self.joined.len() == self.protocol.roles().len() {
            self.status = Status::Open;
        }
    }

    fn matching(&self, role: &str, kind: &str) -> Vec<&Transition> {
        self.protocol
            .transitions()
            .iter()
            .filter(|t| self.knowledge.contains(&t.from) && t.message == kind && t.sender == role)
            .collect()
    }

    /// The conformance check without the signature: everything about the message that depends on
    /// the instance. The signature is checked by [`Instance::check`].
    fn judge(&self, msg: &Message) -> Verdict {
        let hash = msg.hash();
        if self.applied.contains(&hash) {
            return Verdict::Duplicate;
        }
        let b = &msg.body;
        if b.protocol != self.protocol.id().as_str()
            || b.instance.as_deref() != Some(self.id.as_str())
        {
            return Verdict::Refused(Reason::WrongInstance);
        }
        if self.bindings.get(&b.role) != Some(&b.sender) {
            return Verdict::Refused(Reason::RoleNotPermitted);
        }
        if let Some(other) = self.seen_seq.get(&(b.role.clone(), b.seq))
            && *other != hash
        {
            return Verdict::Refused(Reason::Equivocation);
        }
        if b.deps.iter().any(|d| !self.applied.contains(d)) {
            return Verdict::Refused(Reason::MissingDependencies);
        }
        if b.seq <= self.last_seq.get(&b.role).copied().unwrap_or(0) {
            return Verdict::Refused(Reason::Sequence);
        }
        let known_roles: BTreeSet<&String> = self.protocol.roles().iter().collect();
        if b.recipients.is_empty()
            || b.recipients
                .iter()
                .any(|r| !known_roles.contains(r) || *r == b.role)
        {
            return Verdict::Refused(Reason::Recipients);
        }
        match b.kind.as_str() {
            control::GENESIS => Verdict::Refused(Reason::BadGenesis("a second genesis".into())),
            control::JOIN | control::DECLINE => {
                if self.status != Status::Pending {
                    return Verdict::Refused(Reason::NotOpen);
                }
                if self.joined.contains(&b.role) {
                    return Verdict::Refused(Reason::AlreadyJoined);
                }
                if !is_empty_payload(&b.payload) {
                    return Verdict::Refused(Reason::PayloadShape(
                        "a join carries no payload".into(),
                    ));
                }
                Verdict::Accepted
            }
            control::VIOLATION => Verdict::Accepted,
            control::COMPLETE => {
                if self.status == Status::Open && self.is_complete() {
                    Verdict::Accepted
                } else {
                    Verdict::Refused(Reason::NotEnabledInState)
                }
            }
            kind => {
                if self.status != Status::Open {
                    return Verdict::Refused(Reason::NotOpen);
                }
                let Some(message) = self.protocol.messages().get(kind) else {
                    return Verdict::Refused(Reason::UnknownMessage);
                };
                let own = self.matching(&b.role, kind);
                let Some(t) = own.first() else {
                    let by_other = self.protocol.transitions().iter().any(|t| {
                        self.knowledge.contains(&t.from) && t.message == kind && t.sender != b.role
                    });
                    return Verdict::Refused(if by_other {
                        Reason::RoleNotPermitted
                    } else {
                        Reason::NotEnabledInState
                    });
                };
                if !t.receivers.iter().all(|r| b.recipients.contains(r)) {
                    return Verdict::Refused(Reason::Recipients);
                }
                let shape_result = match &message.shape {
                    Some(s) => s.validate(&b.payload),
                    None if is_empty_payload(&b.payload) => Ok(()),
                    None => Err(crate::shape::ShapeError {
                        path: "payload".into(),
                        reason: "this message has no payload".into(),
                    }),
                };
                match shape_result {
                    Ok(()) => Verdict::Accepted,
                    Err(e) => Verdict::Refused(Reason::PayloadShape(e.to_string())),
                }
            }
        }
    }

    /// The conformance check: a pure function of the instance and the message. It checks, in
    /// order, the signature, the protocol and instance, the sender's role binding, the
    /// dependencies, whether the instance is open, whether the role may send the message type
    /// in a state the participant may be in, and the payload shape.
    // @tg: implements:: [[openspec:protocol-conformance#Conformance Check]]
    // @tg: implements:: [[openspec:protocol-spaces#Role-Derived Authority]]
    #[must_use]
    pub fn check(&self, msg: &Message, verifier: &dyn Verifier) -> Verdict {
        if msg.verify(verifier).is_err() {
            return Verdict::Refused(Reason::Signature);
        }
        self.judge(msg)
    }

    fn visible(&self, msg: &Message) -> bool {
        msg.body.role == self.local_role || msg.body.recipients.contains(&self.local_role)
    }

    fn apply(&mut self, msg: &Message) {
        let hash = msg.hash();
        let b = &msg.body;
        for d in &b.deps {
            self.heads.remove(d);
        }
        self.heads.insert(hash.clone());
        self.applied.insert(hash.clone());
        self.order.push(hash.clone());
        self.messages.insert(hash.clone(), msg.clone());
        self.last_seq.insert(b.role.clone(), b.seq);
        self.seen_seq.insert((b.role.clone(), b.seq), hash);
        match b.kind.as_str() {
            control::JOIN => {
                for (r, d) in self.bindings.clone() {
                    if d == b.sender {
                        self.joined.insert(r);
                    }
                }
                self.refresh_status();
            }
            control::DECLINE => self.status = Status::Declined,
            control::COMPLETE => self.status = Status::Completed,
            control::VIOLATION => {}
            kind => {
                let next: BTreeSet<String> = if self.visible(msg) {
                    self.matching(&b.role, kind)
                        .iter()
                        .map(|t| t.to.clone())
                        .collect()
                } else {
                    self.protocol
                        .transitions()
                        .iter()
                        .filter(|t| self.knowledge.contains(&t.from))
                        .map(|t| t.to.clone())
                        .collect()
                };
                if !next.is_empty() {
                    self.knowledge = self.closure(&self.local_role.clone(), next);
                }
            }
        }
    }

    /// Records a message hidden from this participant, known by its hash only, and applies any
    /// held message that was waiting for it. The hidden message counts as one unknown step of
    /// the choreography.
    // @tg: implements:: [[openspec:protocol-spaces#Hash-Linked History]]
    pub fn observe_hidden(&mut self, hash: &str, verifier: &dyn Verifier) -> Vec<Event> {
        let mut events = Vec::new();
        if self.applied.insert(hash.to_owned()) {
            self.order.push(hash.to_owned());
            self.heads.insert(hash.to_owned());
            let next: BTreeSet<String> = self
                .protocol
                .transitions()
                .iter()
                .filter(|t| self.knowledge.contains(&t.from))
                .map(|t| t.to.clone())
                .collect();
            if !next.is_empty() {
                self.knowledge = self.closure(&self.local_role.clone(), next);
            }
            self.drain(verifier, &mut events);
        }
        events
    }

    /// Takes in a message from the network: verifies it, applies it, holds it for its
    /// dependencies, or refuses it, and then applies whatever held messages it unblocked.
    // @tg: implements:: [[openspec:protocol-conformance#Causal Delivery]]
    // @tg: implements:: [[openspec:protocol-conformance#Violation Records]]
    pub fn receive(&mut self, msg: &Message, verifier: &dyn Verifier, now_ms: u64) -> Vec<Event> {
        let hash = msg.hash();
        let mut events = Vec::new();
        match self.check(msg, verifier) {
            Verdict::Duplicate => events.push(Event::Duplicate(hash)),
            Verdict::Accepted => {
                self.apply(msg);
                events.push(Event::Applied(hash));
                self.drain(verifier, &mut events);
            }
            Verdict::Refused(Reason::MissingDependencies) => {
                if self.held.iter().any(|h| h.message.hash() == hash) {
                    events.push(Event::Held(hash));
                } else {
                    if self.held.len() >= self.protocol.limits().buffer_size as usize {
                        let evicted = self.held.remove(0);
                        events.push(Event::Refused {
                            hash: evicted.message.hash(),
                            reason: Reason::BufferFull,
                            message: Box::new(evicted.message),
                        });
                    }
                    self.held.push(Held {
                        message: msg.clone(),
                        arrived_ms: now_ms,
                    });
                    events.push(Event::Held(hash));
                }
            }
            Verdict::Refused(reason) => {
                if reason == Reason::Equivocation
                    && let Some(prev) = self.seen_seq.get(&(msg.body.role.clone(), msg.body.seq))
                    && let Some(first) = self.messages.get(prev)
                {
                    events.push(Event::Equivocation(Box::new(Equivocation {
                        a: first.clone(),
                        b: msg.clone(),
                    })));
                }
                events.push(Event::Refused {
                    hash,
                    reason,
                    message: Box::new(msg.clone()),
                });
            }
        }
        events
    }

    fn drain(&mut self, verifier: &dyn Verifier, events: &mut Vec<Event>) {
        loop {
            let mut progressed = false;
            let mut i = 0;
            while i < self.held.len() {
                let candidate = self.held[i].message.clone();
                match self.check(&candidate, verifier) {
                    Verdict::Refused(Reason::MissingDependencies) => i += 1,
                    Verdict::Accepted => {
                        self.held.remove(i);
                        self.apply(&candidate);
                        events.push(Event::Applied(candidate.hash()));
                        progressed = true;
                    }
                    Verdict::Duplicate => {
                        self.held.remove(i);
                        events.push(Event::Duplicate(candidate.hash()));
                    }
                    Verdict::Refused(reason) => {
                        self.held.remove(i);
                        events.push(Event::Refused {
                            hash: candidate.hash(),
                            reason,
                            message: Box::new(candidate),
                        });
                    }
                }
            }
            if !progressed {
                return;
            }
        }
    }

    /// Drops held messages that have waited past the protocol's timeout and reports each as a
    /// stall, so the chart can take its cancel path.
    // @tg: implements:: [[openspec:protocol-conformance#Causal Delivery]]
    pub fn expire(&mut self, now_ms: u64) -> Vec<Event> {
        let timeout = self.protocol.limits().timeout_ms;
        let mut events = Vec::new();
        let mut keep = Vec::new();
        for h in std::mem::take(&mut self.held) {
            if now_ms.saturating_sub(h.arrived_ms) >= timeout {
                let missing = h
                    .message
                    .body
                    .deps
                    .iter()
                    .filter(|d| !self.applied.contains(*d))
                    .cloned()
                    .collect();
                events.push(Event::Stalled {
                    hash: h.message.hash(),
                    missing,
                });
            } else {
                keep.push(h);
            }
        }
        self.held = keep;
        events
    }

    /// Builds, checks, signs and applies a message of this participant's role. The check is the
    /// same one a receiver runs, on the participant's own projection, so a message the role may
    /// not send never leaves.
    ///
    /// # Errors
    ///
    /// [`SendError::Refused`] with the reason the role may not send it here, or
    /// [`SendError::Sign`].
    // @tg: implements:: [[openspec:protocol-spaces#Chart Role Binding]] {confidence: partial, note: "the outbound check; the mailbox and chart binding live in vertep-federation"}
    pub fn send(
        &mut self,
        signer: &dyn Signer,
        kind: &str,
        payload: Value,
        recipients: &[&str],
    ) -> Result<Message, SendError> {
        let body = Body {
            protocol: self.protocol.id().as_str().to_owned(),
            instance: Some(self.id.clone()),
            sender: signer.did().to_owned(),
            role: self.local_role.clone(),
            seq: self.last_seq.get(&self.local_role).copied().unwrap_or(0) + 1,
            deps: self.heads.iter().cloned().collect(),
            recipients: recipients.iter().map(|r| (*r).to_owned()).collect(),
            kind: kind.to_owned(),
            payload,
        };
        let mut probe = Message {
            body: body.clone(),
            key: String::new(),
            signature: String::new(),
        };
        probe.body.normalize();
        match self.judge(&probe) {
            Verdict::Accepted => {}
            Verdict::Duplicate => return Err(SendError::Refused(Reason::Sequence)),
            Verdict::Refused(r) => return Err(SendError::Refused(r)),
        }
        let msg = Message::sign(body, signer).map_err(SendError::Sign)?;
        self.apply(&msg);
        Ok(msg)
    }

    /// Joins the instance: a signed `join` to every other role.
    ///
    /// # Errors
    ///
    /// As [`Instance::send`].
    // @tg: implements:: [[openspec:protocol-spaces#Joining]]
    pub fn join(&mut self, signer: &dyn Signer) -> Result<Message, SendError> {
        let others: Vec<String> = self
            .protocol
            .roles()
            .iter()
            .filter(|r| **r != self.local_role)
            .cloned()
            .collect();
        let refs: Vec<&str> = others.iter().map(String::as_str).collect();
        self.send(signer, control::JOIN, json!({}), &refs)
    }

    /// Declines the instance: a signed `decline` to every other role.
    ///
    /// # Errors
    ///
    /// As [`Instance::send`].
    // @tg: implements:: [[openspec:protocol-spaces#Joining]]
    pub fn decline(&mut self, signer: &dyn Signer) -> Result<Message, SendError> {
        let others: Vec<String> = self
            .protocol
            .roles()
            .iter()
            .filter(|r| **r != self.local_role)
            .cloned()
            .collect();
        let refs: Vec<&str> = others.iter().map(String::as_str).collect();
        self.send(signer, control::DECLINE, json!({}), &refs)
    }

    /// The history as `role` may read it, in the order it was applied: a message the role sent
    /// or was sealed to in full, any other as its hash only.
    // @tg: implements:: [[openspec:protocol-spaces#Hash-Linked History]]
    #[must_use]
    pub fn history_for(&self, role: &str) -> Vec<HistoryEntry> {
        self.order
            .iter()
            .map(|h| match self.messages.get(h) {
                Some(m) if m.body.role == role || m.body.recipients.iter().any(|r| r == role) => {
                    HistoryEntry::Full(Box::new(m.clone()))
                }
                _ => HistoryEntry::Hidden(h.clone()),
            })
            .collect()
    }

    /// Rebuilds an instance for `local_did` from a forwarded history. Every full entry goes
    /// through the same checks as a live message, so a doctored history is refused, and every
    /// hidden entry keeps the hash chain intact.
    ///
    /// # Errors
    ///
    /// [`InstanceError`] naming the first entry that fails.
    // @tg: implements:: [[openspec:protocol-spaces#Hash-Linked History]]
    pub fn from_history(
        protocol: Arc<Protocol>,
        entries: &[HistoryEntry],
        local_did: &str,
        verifier: &dyn Verifier,
    ) -> Result<Self, InstanceError> {
        let mut iter = entries.iter().enumerate();
        let Some((_, HistoryEntry::Full(genesis))) = iter.next() else {
            return Err(InstanceError(
                "the history does not start with a readable genesis".into(),
            ));
        };
        let mut inst = Self::from_genesis(protocol, genesis, local_did, verifier, None)
            .map_err(|r| InstanceError(format!("entry 0: {r}")))?;
        for (i, e) in iter {
            match e {
                HistoryEntry::Hidden(h) => {
                    inst.observe_hidden(h, verifier);
                }
                HistoryEntry::Full(m) => {
                    for ev in inst.receive(m, verifier, 0) {
                        if let Event::Refused { reason, .. } = ev {
                            return Err(InstanceError(format!("entry {i}: {reason}")));
                        }
                    }
                    if !inst.has_applied(&m.hash()) {
                        return Err(InstanceError(format!(
                            "entry {i}: depends on a message not earlier in the history"
                        )));
                    }
                }
            }
        }
        Ok(inst)
    }
}
