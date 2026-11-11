//! The relay outbox: cross-trust sends from the runtime, with outbound grants, transport
//! selection from the recipient's DID document, fallback, background retries and
//! `error.communication` on failure.
//!
//! The runtime resolves a non-local `did:` address through [`RelayOutbox`] (a runtime
//! [`Resolver`]), and its outbox hands the send to an [`OutboundMailbox`], which only queues it:
//! the chart's turn never waits on the network. [`RelayOutbox::pump`] then works through the
//! queue. A send whose sender holds no grant for the target, whose DID cannot be resolved or has
//! no compatible service, or that a relay refuses, fails at once; one whose endpoints are all
//! unreachable is retried with backoff until the [`RetryPolicy`] is exhausted. Every failure
//! becomes `error.communication` in the sending chart, carrying the send id and the reason. The
//! queue entry stays pending until the send succeeds or fails, and in a store-backed queue it
//! survives a restart; the envelope keeps the runtime's message id, so a re-send is a duplicate
//! the receiver drops.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

use vertep_runtime::mailbox::MailboxError;
use vertep_runtime::transport::TransportError;
use vertep_runtime::{Address, Envelope, Mailbox, NodeHandle, Resolver, Transport};

use crate::did::{Did, is_did_address};
use crate::directory::{DidDirectory, ResolvedAddress, TransportKind};
use crate::envelope::RelayEnvelope;
use crate::error::{CommunicationError, CommunicationFailure};
use crate::federation::Identities;
use crate::grants::GrantSet;
use crate::net::RelayNetwork;
use crate::relay::{RelayProtocol, SendContext, SendError};
use crate::resolver::CachingResolver;
use crate::time::Clock;

/// How long a cross-trust send keeps trying when its endpoints are unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts before the send fails (each attempt tries every compatible route).
    pub max_attempts: u32,
    /// The wait after the first failed attempt, in milliseconds; it doubles per attempt.
    pub backoff_ms: i64,
    /// The longest wait between attempts, in milliseconds.
    pub max_backoff_ms: i64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            max_attempts: 5,
            backoff_ms: 1_000,
            max_backoff_ms: 60_000,
        }
    }
}

impl RetryPolicy {
    /// The wait after failed attempt number `attempt` (1-based).
    pub fn backoff(&self, attempt: u32) -> i64 {
        let shift = attempt.saturating_sub(1).min(30);
        self.backoff_ms
            .saturating_mul(1_i64 << shift)
            .min(self.max_backoff_ms)
    }
}

/// What happened to one cross-trust send attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundOutcome {
    /// Handed to a relay or the recipient.
    Sent {
        /// The transport used.
        transport: TransportKind,
        /// The DID document service used.
        service_id: String,
        /// The services tried first and why each failed: the recorded fallback.
        fell_back_from: Vec<(String, String)>,
    },
    /// Every route failed for now; retried later.
    Retrying {
        /// The failed attempt's number.
        attempt: u32,
        /// The last route's failure.
        reason: String,
        /// When the next attempt runs (milliseconds since the epoch).
        next_at: i64,
    },
    /// Given up: `error.communication` was raised in the sender.
    Failed {
        /// The reason reported to the sender.
        reason: String,
    },
}

/// The record of one outbound attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundRecord {
    /// The message id.
    pub message_id: String,
    /// The `sendid` of the chart's `<send>`.
    pub sendid: Option<String>,
    /// The sending actor's address.
    pub from: Option<String>,
    /// The recipient DID.
    pub to: String,
    /// The outcome.
    pub outcome: OutboundOutcome,
    /// When (milliseconds since the epoch).
    pub at: i64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Attempts {
    count: u32,
    next_at: i64,
    forced: Option<TransportKind>,
    system: bool,
}

/// The shared state of the outbox, its mailboxes and its transports.
pub(crate) struct OutboxCore {
    pub(crate) queue: Arc<dyn Mailbox>,
    attempts: Mutex<HashMap<String, Attempts>>,
    pub(crate) directory: DidDirectory,
    pub(crate) resolver: Arc<CachingResolver>,
    pub(crate) protocols: BTreeMap<TransportKind, Arc<dyn RelayProtocol>>,
    pub(crate) net: Arc<dyn RelayNetwork>,
    pub(crate) identities: Arc<Identities>,
    pub(crate) grants: RwLock<GrantSet>,
    pub(crate) policy: RetryPolicy,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) handle: RwLock<Option<NodeHandle>>,
    records: Mutex<Vec<OutboundRecord>>,
    /// `(sender, recipient)` pairs that have exchanged a message: a reply path for errors.
    pub(crate) contacted: RwLock<BTreeSet<(String, String)>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl OutboxCore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        queue: Arc<dyn Mailbox>,
        directory: DidDirectory,
        resolver: Arc<CachingResolver>,
        protocols: BTreeMap<TransportKind, Arc<dyn RelayProtocol>>,
        net: Arc<dyn RelayNetwork>,
        identities: Arc<Identities>,
        policy: RetryPolicy,
        clock: Arc<dyn Clock>,
    ) -> Self {
        OutboxCore {
            queue,
            attempts: Mutex::new(HashMap::new()),
            directory,
            resolver,
            protocols,
            net,
            identities,
            grants: RwLock::new(GrantSet::new()),
            policy,
            clock,
            handle: RwLock::new(None),
            records: Mutex::new(Vec::new()),
            contacted: RwLock::new(BTreeSet::new()),
        }
    }

    fn record(&self, env: &Envelope, outcome: OutboundOutcome) {
        lock(&self.records).push(OutboundRecord {
            message_id: env.message_id.clone(),
            sendid: env.event.sendid.clone(),
            from: env.event.origin.clone(),
            to: env.to.clone(),
            outcome,
            at: self.clock.now().millis(),
        });
    }

    pub(crate) fn records(&self) -> Vec<OutboundRecord> {
        lock(&self.records).clone()
    }

    /// The outbound grant check: `Err` when the sender may not send this event to the target.
    fn check_grant(&self, env: &Envelope) -> Result<(), CommunicationError> {
        let sender = env.event.origin.clone().unwrap_or_default();
        let g = self.grants.read().unwrap_or_else(|e| e.into_inner());
        g.check_outbound(
            &sender,
            &env.to,
            &env.event.name,
            env.event.sendid.as_deref(),
            self.clock.now(),
        )
        .map(|_| ())
    }

    /// Queues a send. An ungranted send fails here, before anything is sealed or queued.
    pub(crate) fn enqueue(&self, env: Envelope, forced: Option<TransportKind>, system: bool) {
        if !system && let Err(e) = self.check_grant(&env) {
            self.fail(&env, e.failure);
            return;
        }
        lock(&self.attempts).insert(
            env.message_id.clone(),
            Attempts {
                count: 0,
                next_at: i64::MIN,
                forced,
                system,
            },
        );
        if let Err(e) = self.queue.enqueue(env.clone()) {
            self.fail(&env, CommunicationFailure::SealFailed(e.to_string()));
        }
    }

    /// Raises `error.communication` in the sender and records the failure.
    fn fail(&self, env: &Envelope, failure: CommunicationFailure) {
        let mut err = CommunicationError::new(failure);
        if let Some(id) = &env.event.sendid {
            err = err.for_send(id.clone());
        }
        self.record(
            env,
            OutboundOutcome::Failed {
                reason: err.failure.to_string(),
            },
        );
        let Some(sender) = env.event.origin.as_deref() else {
            return;
        };
        let Ok(to) = Address::parse(sender) else {
            return;
        };
        let handle = self.handle.read().ok().and_then(|h| h.clone());
        if let Some(h) = handle {
            let mut error = Envelope::new(format!("{}/error", env.message_id), &to, err.to_event());
            error.correlation.clone_from(&env.correlation);
            // A sender that is not hosted here (a reply to a remote refusal) is not notified.
            let _ = h.send_envelope(error);
        }
    }

    /// One attempt over every compatible route, in preference order.
    fn attempt(
        &self,
        env: &Envelope,
        forced: Option<TransportKind>,
    ) -> Result<(), (SendError, bool)> {
        let resolved = self
            .directory
            .resolve(&env.to)
            .map_err(|e| (SendError::Failed(e.failure), false))?;
        match self.try_routes(env, &resolved, forced) {
            Err((SendError::Stale(_), _)) => {
                // The cached document is stale: resolve once more and retry with the fresh one.
                if let Ok(did) = Did::parse(&env.to) {
                    self.resolver.invalidate(&did);
                }
                let fresh = self
                    .directory
                    .resolve(&env.to)
                    .map_err(|e| (SendError::Failed(e.failure), false))?;
                self.try_routes(env, &fresh, forced)
            }
            other => other,
        }
    }

    /// Tries the routes in order. On failure, returns the last error and whether every route
    /// failed only by being unreachable (worth retrying later).
    fn try_routes(
        &self,
        env: &Envelope,
        to: &ResolvedAddress,
        forced: Option<TransportKind>,
    ) -> Result<(), (SendError, bool)> {
        let routes: Vec<_> = to
            .routes
            .iter()
            .filter(|r| forced.is_none_or(|k| k == r.transport))
            .filter(|r| self.protocols.contains_key(&r.transport))
            .collect();
        if routes.is_empty() {
            return Err((
                SendError::Failed(CommunicationFailure::NoCompatibleEndpoint),
                false,
            ));
        }
        let mut relay = RelayEnvelope::from_event(env.message_id.clone(), &env.event);
        relay.correlation.clone_from(&env.correlation);
        let sender = env
            .event
            .origin
            .as_deref()
            .and_then(|s| self.identities.actor(s));
        let mut fell_back_from = Vec::new();
        let mut last = None;
        let mut retryable = false;
        for route in routes {
            let Some(protocol) = self.protocols.get(&route.transport) else {
                continue;
            };
            let cx = SendContext {
                from: sender.as_ref(),
                to,
                route,
                envelope: &relay,
                net: self.net.as_ref(),
                resolver: self.resolver.as_ref(),
            };
            match protocol.send(&cx) {
                Ok(()) => {
                    self.record(
                        env,
                        OutboundOutcome::Sent {
                            transport: route.transport,
                            service_id: route.service_id.clone(),
                            fell_back_from,
                        },
                    );
                    if let Some(s) = &env.event.origin
                        && let Ok(mut c) = self.contacted.write()
                    {
                        c.insert((s.clone(), env.to.clone()));
                    }
                    return Ok(());
                }
                Err(e @ (SendError::Unreachable(_) | SendError::Stale(_))) => {
                    fell_back_from.push((route.service_id.clone(), e.to_string()));
                    retryable = true;
                    last = Some(e);
                }
                Err(e @ SendError::Unsupported(_)) => {
                    fell_back_from.push((route.service_id.clone(), e.to_string()));
                    if last.is_none() {
                        last = Some(e);
                    }
                }
                Err(e) => return Err((e, false)),
            }
        }
        let e = last.unwrap_or(SendError::Failed(
            CommunicationFailure::NoCompatibleEndpoint,
        ));
        Err((e, retryable))
    }

    /// Works through the queued sends that are due. Returns how many finished (sent or failed).
    pub(crate) fn pump(&self) -> usize {
        let now = self.clock.now().millis();
        let mut taken = Vec::new();
        while let Some(env) = self.queue.receive(&[]) {
            taken.push(env);
        }
        let mut finished = 0;
        for env in taken {
            let a = lock(&self.attempts)
                .get(&env.message_id)
                .copied()
                .unwrap_or_default();
            if a.next_at > now {
                self.queue.release(&env.message_id);
                continue;
            }
            if !a.system
                && let Err(e) = self.check_grant(&env)
            {
                self.done(&env);
                self.fail(&env, e.failure);
                finished += 1;
                continue;
            }
            match self.attempt(&env, a.forced) {
                Ok(()) => {
                    self.done(&env);
                    finished += 1;
                }
                Err((e, retry)) => {
                    let count = a.count + 1;
                    if retry && count < self.policy.max_attempts {
                        let next_at = now.saturating_add(self.policy.backoff(count));
                        lock(&self.attempts).insert(
                            env.message_id.clone(),
                            Attempts {
                                count,
                                next_at,
                                ..a
                            },
                        );
                        self.record(
                            &env,
                            OutboundOutcome::Retrying {
                                attempt: count,
                                reason: e.to_string(),
                                next_at,
                            },
                        );
                        self.queue.release(&env.message_id);
                    } else {
                        self.done(&env);
                        let failure = if retry {
                            CommunicationFailure::Unreachable(format!(
                                "{} after {count} attempts: {e}",
                                env.to
                            ))
                        } else {
                            e.to_failure()
                        };
                        if !a.system {
                            self.fail(&env, failure);
                        } else {
                            self.record(
                                &env,
                                OutboundOutcome::Failed {
                                    reason: failure.to_string(),
                                },
                            );
                        }
                        finished += 1;
                    }
                }
            }
        }
        finished
    }

    fn done(&self, env: &Envelope) {
        let _ = self.queue.ack(&env.message_id);
        lock(&self.attempts).remove(&env.message_id);
    }
}

/// The runtime's view of a non-local DID actor: a mailbox that queues sends for the relay
/// outbox. It never yields messages.
pub struct OutboundMailbox {
    core: Arc<OutboxCore>,
    forced: Option<TransportKind>,
}

impl fmt::Debug for OutboundMailbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboundMailbox")
            .field("forced", &self.forced)
            .finish()
    }
}

impl Mailbox for OutboundMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        self.core.enqueue(env, self.forced, false);
        Ok(())
    }

    fn receive(&self, _deferred: &[String]) -> Option<Envelope> {
        None
    }

    fn ack(&self, _message_id: &str) -> Result<(), MailboxError> {
        Ok(())
    }

    fn release(&self, _message_id: &str) {}

    fn release_all(&self) {}

    fn pending(&self) -> Vec<Envelope> {
        Vec::new()
    }

    fn pending_deliverable(&self, _deferred: &[String]) -> usize {
        0
    }
}

/// The relay outbox, registered with the runtime as the resolver of `did:` addresses.
#[derive(Clone)]
// @tg: implements:: [[openspec:relay-mailboxes#Undeliverable Message Reporting]]
// @tg: implements:: [[openspec:relay-mailboxes#Transport Selection From The DID Document]]
// @tg: implements:: [[openspec:did-identity#Outbound Grant Enforcement]]
// @tg: implements:: [[openspec:did-identity#DID Resolution As Address Resolution]]
pub struct RelayOutbox {
    pub(crate) core: Arc<OutboxCore>,
}

impl fmt::Debug for RelayOutbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayOutbox")
            .field("policy", &self.core.policy)
            .finish_non_exhaustive()
    }
}

impl RelayOutbox {
    /// Works through the due sends; returns how many finished.
    pub fn pump(&self) -> usize {
        self.core.pump()
    }

    /// The sends still pending (queued or waiting for a retry).
    pub fn pending(&self) -> Vec<Envelope> {
        self.core.queue.pending()
    }

    /// Every attempt so far.
    pub fn records(&self) -> Vec<OutboundRecord> {
        self.core.records()
    }

    /// Queues `env` directly (what the runtime's resolver path does).
    pub fn send(&self, env: Envelope) {
        self.core.enqueue(env, None, false);
    }
}

impl Resolver for RelayOutbox {
    fn resolve(&self, address: &Address) -> Option<Arc<dyn Mailbox>> {
        is_did_address(address.as_str()).then(|| {
            Arc::new(OutboundMailbox {
                core: self.core.clone(),
                forced: None,
            }) as Arc<dyn Mailbox>
        })
    }
}

/// A relay protocol registered as an SCXML Event I/O Processor, so `<send type>` can name it
/// and `_ioprocessors` lists it. A send of this type to a non-local DID uses only this
/// transport; a local recipient gets the message in its mailbox directly.
pub struct RelayTransport {
    core: Arc<OutboxCore>,
    kind: TransportKind,
}

impl fmt::Debug for RelayTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayTransport")
            .field("kind", &self.kind)
            .finish()
    }
}

impl RelayTransport {
    pub(crate) fn new(core: Arc<OutboxCore>, kind: TransportKind) -> Self {
        RelayTransport { core, kind }
    }
}

impl Transport for RelayTransport {
    fn type_iri(&self) -> &str {
        self.kind.io_processor()
    }

    fn aliases(&self) -> Vec<String> {
        vec![
            match self.kind {
                TransportKind::Tsp => "tsp",
                TransportKind::DidComm => "didcomm",
            }
            .to_owned(),
        ]
    }

    fn deliver(&self, mailbox: &dyn Mailbox, env: Envelope) -> Result<(), TransportError> {
        if is_did_address(&env.to) && !self.core.identities.is_local(&env.to) {
            self.core.enqueue(env, Some(self.kind), false);
            Ok(())
        } else {
            mailbox
                .enqueue(env)
                .map_err(|e| TransportError(e.to_string()))
        }
    }
}
