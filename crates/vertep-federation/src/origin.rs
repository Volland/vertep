//! Verified origins and the inbound admission pipeline the shell runs before a journal append.
//!
//! A transport that opens a message reports a [`VerifiedOrigin`]: the sender DID only when the
//! sender's authenticated encryption or signature verified against its resolved DID document,
//! and the I/O processor that delivered it. [`admit`] then applies the actor's anonymous-sender
//! policy, its grants and its declared payload types, in that order, and returns either the
//! event to journal or a [`Refusal`] to record. Deduplication by message id stays with the
//! journal.
//!
//! The verification outcome is an input (design D4): the runtime journals the
//! [`VerifiedOrigin`] with the event and replays from it without re-verifying. `vertep-core`'s
//! `_event` gains one field for the verified flag; until it exists, [`VerifiedOrigin::apply`]
//! sets `origin` and `origintype` only.

use std::fmt;

use serde::{Deserialize, Serialize};
use vertep_core::datamodel::{Event, Value};
use vertep_core::types::{Declarations, EventTable};

use crate::envelope::RelayEnvelope;
use crate::grants::{GrantRefusal, GrantSet};
use crate::time::Timestamp;

/// Who sent a message, as established by the transport that opened it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedOrigin {
    /// The sender DID when `verified`; `None` for an anonymous sender.
    pub sender: Option<String>,
    /// True when the sender's authentication verified against its DID document.
    pub verified: bool,
    /// The verification method that authenticated the sender.
    pub sender_key: Option<String>,
    /// The Event I/O Processor type IRI that delivered the message (`_event.origintype`).
    pub io_processor: String,
}

impl VerifiedOrigin {
    /// An authenticated sender.
    pub fn verified(
        sender: impl Into<String>,
        sender_key: impl Into<String>,
        io_processor: impl Into<String>,
    ) -> Self {
        VerifiedOrigin {
            sender: Some(sender.into()),
            verified: true,
            sender_key: Some(sender_key.into()),
            io_processor: io_processor.into(),
        }
    }

    /// An unauthenticated (anonymously encrypted) sender.
    pub fn anonymous(io_processor: impl Into<String>) -> Self {
        VerifiedOrigin {
            sender: None,
            verified: false,
            sender_key: None,
            io_processor: io_processor.into(),
        }
    }

    /// Sets `_event.origin` to the verified sender DID (or clears it) and `_event.origintype`
    /// to the I/O processor. Nothing from the envelope body reaches these fields.
    // @tg: implements:: [[openspec:did-identity#Authenticated Event Origin]]
    pub fn apply(&self, event: &mut Event) {
        event.origin = if self.verified {
            self.sender.clone()
        } else {
            None
        };
        event.origintype = Some(self.io_processor.clone());
    }
}

/// Whether an actor accepts messages from unauthenticated senders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnonymousPolicy {
    /// Refuse them (the default).
    #[default]
    Refuse,
    /// Deliver them, marked unverified, for guards to decide.
    Allow,
}

/// A receiving actor's inbound rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InboundPolicy {
    /// The actor's address (its DID).
    pub actor: String,
    /// The anonymous-sender policy.
    pub anonymous: AnonymousPolicy,
    /// The grants the actor holds (verified senders need one).
    pub grants: GrantSet,
}

/// Checks a received payload against the receiving chart's declared event types.
pub trait PayloadTypes {
    /// `Ok` when `data` fits the declared payload type of `event`.
    ///
    /// # Errors
    ///
    /// A description of the mismatch.
    fn check(&self, event: &str, data: &Value) -> Result<(), String>;
}

/// No declared types: every payload is accepted (dynamic charts).
#[derive(Clone, Copy, Debug, Default)]
pub struct Untyped;

impl PayloadTypes for Untyped {
    fn check(&self, _event: &str, _data: &Value) -> Result<(), String> {
        Ok(())
    }
}

/// The declared event types of a chart.
#[derive(Clone, Debug)]
pub struct DeclaredPayloads {
    decls: Declarations,
    table: EventTable,
}

impl DeclaredPayloads {
    /// The payload types `decls` declares.
    pub fn new(decls: Declarations) -> Self {
        let table = EventTable::new(&decls);
        DeclaredPayloads { decls, table }
    }
}

impl PayloadTypes for DeclaredPayloads {
    fn check(&self, event: &str, data: &Value) -> Result<(), String> {
        match self.table.payload_of(event) {
            Some(Some(ty)) => self.decls.check(&ty, data).map_err(|errs| {
                errs.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
            Some(None) if !matches!(data, Value::Undefined | Value::Null) => {
                Err(format!("event {event} declares no payload"))
            }
            _ => Ok(()),
        }
    }
}

/// Why an inbound message was not delivered. Every refusal is recorded, never silently dropped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// The message could not be decrypted or authenticated (tampered, wrong key, key retired
    /// beyond its grace period).
    Undecryptable {
        /// The reason.
        reason: String,
        /// The key id the message was addressed to, if known.
        kid: Option<String>,
        /// The sender, if the envelope names one.
        sender: Option<String>,
    },
    /// The message claims a sender whose keys do not authenticate it.
    ForgedOrigin {
        /// The claimed sender.
        claimed: String,
        /// The reason.
        reason: String,
    },
    /// An unauthenticated sender, and the actor requires authentication.
    AnonymousRefused,
    /// The sender holds no grant for the event.
    NotGranted(String),
    /// The payload does not fit the event's declared type.
    PayloadType {
        /// The event name.
        event: String,
        /// The mismatch.
        reason: String,
    },
    /// The envelope is unreadable.
    MalformedEnvelope(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Undecryptable { reason, .. } => write!(f, "undecryptable: {reason}"),
            Refusal::ForgedOrigin { claimed, reason } => {
                write!(f, "origin {claimed} not authenticated: {reason}")
            }
            Refusal::AnonymousRefused => f.write_str("anonymous sender refused by policy"),
            Refusal::NotGranted(m) => write!(f, "not granted: {m}"),
            Refusal::PayloadType { event, reason } => {
                write!(
                    f,
                    "payload of {event} does not fit its declared type: {reason}"
                )
            }
            Refusal::MalformedEnvelope(m) => write!(f, "malformed envelope: {m}"),
        }
    }
}

impl std::error::Error for Refusal {}

impl Refusal {
    /// True when the sender should get `error.communication` (if a reply path exists): the
    /// message was readable and authenticated, but refused.
    pub fn notifies_sender(&self) -> bool {
        matches!(self, Refusal::NotGranted(_) | Refusal::PayloadType { .. })
    }
}

/// A message a transport opened: the envelope and who sent it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenedMessage {
    /// The envelope.
    pub envelope: RelayEnvelope,
    /// The verified origin.
    pub origin: VerifiedOrigin,
}

/// An admitted message: the event to journal with its message id and verification outcome.
#[derive(Clone, Debug, PartialEq)]
pub struct Admitted {
    /// The deduplication key.
    pub message_id: String,
    /// The event as the receiver's `_event` sees it.
    pub event: Event,
    /// The verification outcome, journaled with the event.
    pub origin: VerifiedOrigin,
}

/// The record of one inbound message, accepted or refused, for the journal and the graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundRecord {
    /// The receiving actor.
    pub actor: String,
    /// The message id (absent when the message could not be opened).
    pub message_id: Option<String>,
    /// The event name, when readable.
    pub event: Option<String>,
    /// The verification outcome, when the message opened.
    pub origin: Option<VerifiedOrigin>,
    /// `None` when accepted.
    pub refusal: Option<Refusal>,
    /// When it was received (milliseconds since the epoch).
    pub at: i64,
}

impl InboundRecord {
    /// The record of an admitted message.
    pub fn accepted(actor: &str, a: &Admitted, at: Timestamp) -> Self {
        InboundRecord {
            actor: actor.to_owned(),
            message_id: Some(a.message_id.clone()),
            event: Some(a.event.name.clone()),
            origin: Some(a.origin.clone()),
            refusal: None,
            at: at.millis(),
        }
    }

    /// The record of a refused message.
    pub fn refused(
        actor: &str,
        opened: Option<&OpenedMessage>,
        refusal: Refusal,
        at: Timestamp,
    ) -> Self {
        InboundRecord {
            actor: actor.to_owned(),
            message_id: opened.map(|o| o.envelope.message_id.clone()),
            event: opened.map(|o| o.envelope.name.clone()),
            origin: opened.map(|o| o.origin.clone()),
            refusal: Some(refusal),
            at: at.millis(),
        }
    }
}

/// Decides whether an opened message is delivered to `policy.actor`: anonymous-sender policy,
/// then grants (for authenticated senders), then the payload's declared type.
///
/// # Errors
///
/// The [`Refusal`] to record. Nothing is journaled for a refused message.
// @tg: implements:: [[openspec:did-identity#Unauthenticated Event Marking]]
// @tg: implements:: [[openspec:did-identity#DID-Scoped Capability Grants]]
// @tg: implements:: [[openspec:did-identity#Journaled Verification Outcome]] {note: "the outcome is the admitted event's origin and origin type, journaled with the input"}
// @tg: implements:: [[openspec:relay-mailboxes#SCXML Event Envelope Mapping]]
pub fn admit(
    policy: &InboundPolicy,
    opened: &OpenedMessage,
    types: &dyn PayloadTypes,
    now: Timestamp,
) -> Result<Admitted, Refusal> {
    let env = &opened.envelope;
    let origin = &opened.origin;
    if origin.verified {
        policy
            .grants
            .check_inbound(origin.sender.as_deref(), &policy.actor, &env.name, now)
            .map_err(|r: GrantRefusal| Refusal::NotGranted(r.to_string()))?;
    } else if policy.anonymous == AnonymousPolicy::Refuse {
        return Err(Refusal::AnonymousRefused);
    }
    types
        .check(&env.name, &env.data.0)
        .map_err(|reason| Refusal::PayloadType {
            event: env.name.clone(),
            reason,
        })?;
    let mut event = env.to_event();
    origin.apply(&mut event);
    Ok(Admitted {
        message_id: env.message_id.clone(),
        event,
        origin: origin.clone(),
    })
}
