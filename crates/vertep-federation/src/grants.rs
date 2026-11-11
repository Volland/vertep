//! DID-scoped capability grants: who may send which events to which actor.
//!
//! A [`Grant`] names a grantee, a target and SCXML event descriptors, with an optional expiry.
//! The same statement serves both directions: inbound, the receiving actor checks the verified
//! sender against grants whose target is itself; outbound, the shell checks the sending actor
//! against grants whose target is the external DID. Both checks run before anything is
//! journaled or sealed (design D5), so an ungranted message never costs the receiver a journal
//! append and an ungranted send never produces an envelope.

use std::fmt;

use serde::{Deserialize, Serialize};
use vertep_core::kernel::descriptor_matches;

use crate::error::{CommunicationError, CommunicationFailure};
use crate::time::Timestamp;

/// A grant: `grantee` may send events matching `events` to `target` until `expires`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The grant statement's IRI, when it has one in the graph.
    pub id: Option<String>,
    /// The sender the grant authorises (a DID, or a `vt:` address for outbound grants of
    /// cluster actors).
    pub grantee: String,
    /// The receiving actor.
    pub target: String,
    /// SCXML event descriptors (`order.*`, `quote.request`, `*`).
    pub events: Vec<String>,
    /// Expiry, in milliseconds since the epoch; `None` for no expiry.
    pub expires: Option<i64>,
    /// The DID that issued the grant as a credential; `None` for a grant configured locally.
    pub issuer: Option<String>,
}

impl Grant {
    /// A non-expiring local grant.
    pub fn new(
        grantee: impl Into<String>,
        target: impl Into<String>,
        events: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Grant {
            id: None,
            grantee: grantee.into(),
            target: target.into(),
            events: events.into_iter().map(Into::into).collect(),
            expires: None,
            issuer: None,
        }
    }

    /// The same grant expiring at `at`.
    pub fn expiring(mut self, at: Timestamp) -> Self {
        self.expires = Some(at.millis());
        self
    }

    /// True when one of the grant's descriptors matches `event`.
    pub fn covers(&self, event: &str) -> bool {
        self.events.iter().any(|d| descriptor_matches(d, event))
    }

    /// True when the grant has not expired at `now`.
    pub fn is_live(&self, now: Timestamp) -> bool {
        self.expires.is_none_or(|e| now.millis() < e)
    }
}

/// Why a grant check refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantRefusal {
    /// The sender is unauthenticated, so no grant can name it.
    Unauthenticated,
    /// No grant names this grantee for this target.
    NoGrant {
        /// The sender.
        grantee: String,
        /// The receiver.
        target: String,
    },
    /// Grants exist, but none covers the event.
    EventNotGranted {
        /// The event name.
        event: String,
    },
    /// The only grants covering the event have expired.
    Expired {
        /// The latest expiry among them.
        expired_at: Timestamp,
    },
}

impl fmt::Display for GrantRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GrantRefusal::Unauthenticated => f.write_str("unauthenticated sender"),
            GrantRefusal::NoGrant { grantee, target } => {
                write!(f, "no grant lets {grantee} send to {target}")
            }
            GrantRefusal::EventNotGranted { event } => write!(f, "event {event} is not granted"),
            GrantRefusal::Expired { expired_at } => write!(f, "grant expired at {expired_at}"),
        }
    }
}

impl std::error::Error for GrantRefusal {}

/// The grants an actor holds or has issued.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantSet {
    grants: Vec<Grant>,
}

impl GrantSet {
    /// No grants.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a grant.
    pub fn add(&mut self, grant: Grant) {
        self.grants.push(grant);
    }

    /// The grants.
    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    /// Drops the grants that have expired at `now`.
    pub fn remove_expired(&mut self, now: Timestamp) {
        self.grants.retain(|g| g.is_live(now));
    }

    /// The live grant that lets `grantee` send `event` to `target` at `now`.
    ///
    /// # Errors
    ///
    /// [`GrantRefusal`] naming why no grant applies.
    // @tg: implements:: [[openspec:did-identity#DID-Scoped Capability Grants]]
    pub fn check(
        &self,
        grantee: &str,
        target: &str,
        event: &str,
        now: Timestamp,
    ) -> Result<&Grant, GrantRefusal> {
        let mut named = false;
        let mut expired: Option<i64> = None;
        for g in self
            .grants
            .iter()
            .filter(|g| g.grantee == grantee && g.target == target)
        {
            named = true;
            if g.covers(event) {
                if g.is_live(now) {
                    return Ok(g);
                }
                expired = expired.max(g.expires);
            }
        }
        if !named {
            return Err(GrantRefusal::NoGrant {
                grantee: grantee.to_owned(),
                target: target.to_owned(),
            });
        }
        match expired {
            Some(at) => Err(GrantRefusal::Expired {
                expired_at: Timestamp(at),
            }),
            None => Err(GrantRefusal::EventNotGranted {
                event: event.to_owned(),
            }),
        }
    }

    /// The inbound check: may the verified sender `sender` (`None` when unauthenticated) send
    /// `event` to `target`?
    ///
    /// # Errors
    ///
    /// [`GrantRefusal`]; an unauthenticated sender is always refused.
    // @tg: implements:: [[openspec:did-identity#DID-Scoped Capability Grants]]
    pub fn check_inbound(
        &self,
        sender: Option<&str>,
        target: &str,
        event: &str,
        now: Timestamp,
    ) -> Result<&Grant, GrantRefusal> {
        let sender = sender.ok_or(GrantRefusal::Unauthenticated)?;
        self.check(sender, target, event, now)
    }

    /// The outbound check: may the actor `sender` send `event` to the cross-trust `target`?
    ///
    /// # Errors
    ///
    /// The `error.communication` failure to raise in the sender, carrying `send_id`.
    // @tg: implements:: [[openspec:did-identity#Outbound Grant Enforcement]]
    pub fn check_outbound(
        &self,
        sender: &str,
        target: &str,
        event: &str,
        send_id: Option<&str>,
        now: Timestamp,
    ) -> Result<&Grant, CommunicationError> {
        self.check(sender, target, event, now).map_err(|r| {
            let e = CommunicationError::new(CommunicationFailure::NotGranted(r.to_string()));
            match send_id {
                Some(id) => e.for_send(id),
                None => e,
            }
        })
    }

    /// Runs `seal` (which encrypts and hands the message to the transport) only when the
    /// outbound grant check passes, so an ungranted send never produces an envelope.
    ///
    /// # Errors
    ///
    /// The grant failure, or the failure `seal` returns.
    // @tg: implements:: [[openspec:did-identity#Outbound Grant Enforcement]]
    #[allow(clippy::too_many_arguments)]
    pub fn send_if_granted<T>(
        &self,
        sender: &str,
        target: &str,
        event: &str,
        send_id: Option<&str>,
        now: Timestamp,
        seal: impl FnOnce() -> Result<T, CommunicationError>,
    ) -> Result<T, CommunicationError> {
        self.check_outbound(sender, target, event, send_id, now)?;
        seal()
    }
}
