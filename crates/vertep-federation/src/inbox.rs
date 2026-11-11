//! Relay mailboxes: the runtime mailboxes of local DID actors, fed by pickups from DIDComm
//! mediators, TSP intermediaries and the node's own inbound queue.
//!
//! A [`RelayInbox`] holds one or more pickup bindings. A shared binding fetches for every local
//! DID actor of the inbox (one session per node or shard) and demultiplexes by recipient; a
//! dedicated binding fetches only for one actor. Each picked-up message is opened through its
//! protocol, admitted (anonymous-sender policy, grants, payload type) and placed in the
//! recipient's [`RelayMailbox`] with the verified origin set; the node is told so the actor is
//! activated. A refused message is recorded and acknowledged to the relay at once.
//!
//! An admitted message is acknowledged to the relay only when the runtime acknowledges it in the
//! mailbox, which it does after the journal append (or when it recognises the message id as a
//! duplicate). A crash in between leaves the copy at the relay; it is fetched again after
//! recovery and dropped as a duplicate by the journal. A relay that redelivers a copy still held
//! here does not create a second row; a second copy of the same message id is attached to the
//! existing row and acknowledged with it.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};

use vertep_core::kernel::descriptor_matches;
use vertep_runtime::mailbox::{MailboxError, MemoryMailbox};
use vertep_runtime::{Address, Envelope, Mailbox, MailboxFactory, NodeHandle, Storage};

use crate::directory::TransportKind;
use crate::error::{CommunicationError, CommunicationFailure};
use crate::federation::Identities;
use crate::net::{NetError, PickupSource, StoredMessage};
use crate::origin::{InboundRecord, OpenedMessage, Refusal, admit};
use crate::outbox::OutboxCore;
use crate::relay::RelayProtocol;
use crate::resolver::DidResolver;
use crate::time::Clock;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Which actors a pickup binding fetches for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipients {
    /// Every local DID actor of the inbox: one mailbox at the relay shared by a node or shard.
    Shared,
    /// One actor's own mailbox at the relay.
    Dedicated(String),
}

/// A relay to pick up from, the protocol its messages use, and whom it fetches for.
#[derive(Clone)]
pub struct PickupBinding {
    /// A name for records.
    pub name: String,
    /// The relay.
    pub source: Arc<dyn PickupSource>,
    /// The protocol that opens its messages.
    pub protocol: Arc<dyn RelayProtocol>,
    /// Whom it fetches for.
    pub recipients: Recipients,
}

impl fmt::Debug for PickupBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PickupBinding")
            .field("name", &self.name)
            .field("protocol", &self.protocol.kind())
            .field("recipients", &self.recipients)
            .finish()
    }
}

struct Picked {
    env: Envelope,
    receipts: Vec<(usize, String)>,
    in_flight: bool,
    seq: u64,
}

/// Arrival order across the picked-up rows and the local queue.
#[derive(Default)]
struct Arrivals {
    next: u64,
    /// Arrival numbers of local messages, by message id (absent: stored before this process).
    local: std::collections::HashMap<String, u64>,
    /// Local messages handed out and not yet acknowledged or released.
    local_in_flight: HashSet<String>,
}

/// The mailbox of a local DID actor: messages picked up from relays, in the relay's order,
/// plus messages from senders on this node, which never leave it, delivered in the order they
/// arrived here.
// @tg: implements:: [[openspec:relay-mailboxes#DIDComm Mediator Mailbox]]
// @tg: implements:: [[openspec:relay-mailboxes#Mailbox Ordering]]
pub struct RelayMailbox {
    address: String,
    local: Arc<dyn Mailbox>,
    picked: Mutex<VecDeque<Picked>>,
    arrivals: Mutex<Arrivals>,
    inbox: Weak<InboxCore>,
}

impl fmt::Debug for RelayMailbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayMailbox")
            .field("address", &self.address)
            .field("picked", &lock(&self.picked).len())
            .finish()
    }
}

fn deferred_by(deferred: &[String], name: &str) -> bool {
    deferred.iter().any(|d| descriptor_matches(d, name))
}

impl RelayMailbox {
    /// Adds a picked-up message, or attaches the receipt to the row of the same message id.
    /// Returns true when a new row was added.
    fn push(&self, env: Envelope, receipt: (usize, String)) -> bool {
        let mut p = lock(&self.picked);
        if let Some(row) = p.iter_mut().find(|r| r.env.message_id == env.message_id) {
            row.receipts.push(receipt);
            return false;
        }
        let seq = {
            let mut a = lock(&self.arrivals);
            a.next += 1;
            a.next
        };
        p.push_back(Picked {
            env,
            receipts: vec![receipt],
            in_flight: false,
            seq,
        });
        true
    }

    fn holds_picked(&self) -> bool {
        !lock(&self.picked).is_empty()
    }
}

impl Mailbox for RelayMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        {
            let mut a = lock(&self.arrivals);
            a.next += 1;
            let seq = a.next;
            a.local.insert(env.message_id.clone(), seq);
        }
        self.local.enqueue(env)
    }

    /// The oldest deliverable message by arrival here, whether it came through a relay or from
    /// a sender on this node.
    fn receive(&self, deferred: &[String]) -> Option<Envelope> {
        let mut p = lock(&self.picked);
        let mut a = lock(&self.arrivals);
        let local_next = self
            .local
            .pending()
            .into_iter()
            .find(|e| {
                !a.local_in_flight.contains(&e.message_id) && !deferred_by(deferred, &e.event.name)
            })
            .map(|e| a.local.get(&e.message_id).copied().unwrap_or(0));
        let picked = p
            .iter_mut()
            .filter(|r| !r.in_flight && !deferred_by(deferred, &r.env.event.name))
            .min_by_key(|r| r.seq);
        match (picked, local_next) {
            (Some(row), l) if l.is_none_or(|l| row.seq < l) => {
                row.in_flight = true;
                Some(row.env.clone())
            }
            _ => {
                let env = self.local.receive(deferred)?;
                a.local_in_flight.insert(env.message_id.clone());
                Some(env)
            }
        }
    }

    fn ack(&self, message_id: &str) -> Result<(), MailboxError> {
        let row = {
            let mut p = lock(&self.picked);
            p.iter()
                .position(|r| r.env.message_id == message_id)
                .and_then(|i| p.remove(i))
        };
        match row {
            Some(row) => {
                if let Some(inbox) = self.inbox.upgrade() {
                    inbox.acknowledge(&row.receipts);
                }
                Ok(())
            }
            None => {
                {
                    let mut a = lock(&self.arrivals);
                    a.local.remove(message_id);
                    a.local_in_flight.remove(message_id);
                }
                self.local.ack(message_id)
            }
        }
    }

    fn release(&self, message_id: &str) {
        let mut found = false;
        for r in lock(&self.picked).iter_mut() {
            if r.env.message_id == message_id {
                r.in_flight = false;
                found = true;
            }
        }
        if !found {
            lock(&self.arrivals).local_in_flight.remove(message_id);
            self.local.release(message_id);
        }
    }

    fn release_all(&self) {
        for r in lock(&self.picked).iter_mut() {
            r.in_flight = false;
        }
        lock(&self.arrivals).local_in_flight.clear();
        self.local.release_all();
    }

    fn pending(&self) -> Vec<Envelope> {
        let mut out: Vec<Envelope> = lock(&self.picked).iter().map(|r| r.env.clone()).collect();
        out.extend(self.local.pending());
        out
    }

    fn pending_deliverable(&self, deferred: &[String]) -> usize {
        lock(&self.picked)
            .iter()
            .filter(|r| !r.in_flight && !deferred_by(deferred, &r.env.event.name))
            .count()
            + self.local.pending_deliverable(deferred)
    }
}

/// The outcome of one pickup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickupReport {
    /// Messages placed in mailboxes, by recipient.
    pub delivered: BTreeMap<String, usize>,
    /// Copies of messages already in a mailbox (redeliveries), attached to the existing row.
    pub duplicates: usize,
    /// Messages refused and recorded.
    pub refused: usize,
    /// Relays that could not be reached.
    pub unreachable: Vec<String>,
    /// Fetch requests made (one per binding).
    pub fetches: usize,
}

pub(crate) struct InboxCore {
    bindings: Vec<PickupBinding>,
    mailboxes: Mutex<HashMap<String, Arc<RelayMailbox>>>,
    held: Mutex<HashSet<(usize, String)>>,
    identities: Arc<Identities>,
    resolver: Arc<dyn DidResolver>,
    records: Mutex<Vec<InboundRecord>>,
    handle: RwLock<Option<NodeHandle>>,
    outbox: RwLock<Weak<OutboxCore>>,
    clock: Arc<dyn Clock>,
    local: Arc<dyn MailboxFactory>,
    limit: usize,
    storage: Option<Arc<dyn Storage>>,
    me: Weak<InboxCore>,
}

impl InboxCore {
    fn acknowledge(&self, receipts: &[(usize, String)]) {
        let mut by_source: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (s, id) in receipts {
            by_source.entry(*s).or_default().push(id.clone());
        }
        let mut held = lock(&self.held);
        for (s, ids) in by_source {
            if let Some(b) = self.bindings.get(s)
                && b.source.ack(&ids).is_ok()
            {
                for id in ids {
                    held.remove(&(s, id));
                }
            }
        }
    }

    fn mailbox(&self, address: &str) -> Arc<RelayMailbox> {
        lock(&self.mailboxes)
            .entry(address.to_owned())
            .or_insert_with(|| {
                let local = Address::parse(address)
                    .map(|a| self.local.create(&a))
                    .unwrap_or_else(|_| Arc::new(MemoryMailbox::default()));
                Arc::new(RelayMailbox {
                    address: address.to_owned(),
                    local,
                    picked: Mutex::new(VecDeque::new()),
                    arrivals: Mutex::new(Arrivals::default()),
                    inbox: self.me.clone(),
                })
            })
            .clone()
    }

    fn recipients(&self, b: &PickupBinding) -> Vec<String> {
        match &b.recipients {
            Recipients::Dedicated(did) => vec![did.clone()],
            Recipients::Shared => self.identities.dids(),
        }
    }

    /// Keeps the record and writes its projection into graph `<actor>/inbound` of the node's
    /// storage, where auditors query what an actor accepted or refused, from whom.
    fn record(&self, r: InboundRecord) {
        if let Some(storage) = &self.storage
            && let Ok(g) = crate::ontology::project_inbound(&r)
            && let (Ok(actor), Ok(graph)) = (
                oxrdf::NamedNode::new(&r.actor),
                oxrdf::NamedNode::new(format!("{}/inbound", r.actor)),
            )
        {
            let quads: Vec<oxrdf::Quad> = g
                .iter()
                .map(|t| t.into_owned().in_graph(graph.clone()))
                .collect();
            let _ = storage.graph(&actor).insert_quads(&quads);
        }
        lock(&self.records).push(r);
    }

    fn refuse(
        &self,
        actor: &str,
        index: usize,
        stored: &StoredMessage,
        opened: Option<&OpenedMessage>,
        refusal: Refusal,
    ) {
        let now = self.clock.now();
        let notify = refusal.notifies_sender();
        self.record(InboundRecord::refused(actor, opened, refusal.clone(), now));
        // Rejected and recorded: the relay may drop it.
        self.acknowledge(&[(index, stored.id.clone())]);
        if notify
            && let Some(o) = opened
            && o.origin.verified
            && let Some(sender) = &o.origin.sender
        {
            self.reply_error(actor, sender, o, &refusal);
        }
    }

    /// Sends `error.communication` back to a verified sender over the relay, as a system reply
    /// that needs no grant.
    fn reply_error(&self, actor: &str, sender: &str, o: &OpenedMessage, refusal: &Refusal) {
        let Some(outbox) = self.outbox.read().ok().and_then(|w| w.upgrade()) else {
            return;
        };
        let Ok(to) = Address::parse(sender) else {
            return;
        };
        let mut err = CommunicationError::new(CommunicationFailure::Refused(refusal.to_string()));
        if let Some(id) = &o.envelope.sendid {
            err = err.for_send(id.clone());
        }
        let mut ev = err.to_event();
        ev.origin = Some(actor.to_owned());
        let mut env = Envelope::new(format!("{}/refused", o.envelope.message_id), &to, ev);
        env.correlation.clone_from(&o.envelope.correlation);
        outbox.enqueue(env, None, true);
    }

    /// True when `actor` sent to `sender` before, so an error reply from it has a reply path.
    fn is_reply(&self, actor: &str, o: &OpenedMessage) -> bool {
        o.origin.verified
            && o.envelope.name == "error.communication"
            && o.origin.sender.as_deref().is_some_and(|s| {
                self.outbox
                    .read()
                    .ok()
                    .and_then(|w| w.upgrade())
                    .is_some_and(|ob| {
                        ob.contacted
                            .read()
                            .is_ok_and(|c| c.contains(&(actor.to_owned(), s.to_owned())))
                    })
            })
    }

    fn pickup_binding(&self, index: usize, report: &mut PickupReport) {
        let Some(b) = self.bindings.get(index) else {
            return;
        };
        let recipients = self.recipients(b);
        if recipients.is_empty() {
            return;
        }
        report.fetches += 1;
        let batch = match b.source.fetch(&recipients, self.limit) {
            Ok(batch) => batch,
            Err(NetError::Unreachable(m) | NetError::Gone(m) | NetError::Refused(m)) => {
                report.unreachable.push(format!("{}: {m}", b.name));
                return;
            }
        };
        let mut woken = Vec::new();
        for stored in batch {
            let key = (index, stored.id.clone());
            if lock(&self.held).contains(&key) {
                continue;
            }
            let Some(entry) = self.identities.entry(&stored.recipient) else {
                // Not ours (any more): leave it at the relay.
                continue;
            };
            let now = self.clock.now();
            let opened =
                match b
                    .protocol
                    .open(&entry.actor, &stored.bytes, self.resolver.as_ref(), now)
                {
                    Ok(o) => o,
                    Err(refusal) => {
                        self.refuse(&stored.recipient, index, &stored, None, refusal);
                        report.refused += 1;
                        continue;
                    }
                };
            let admitted = if self.is_reply(&stored.recipient, &opened) {
                let mut ev = opened.envelope.to_event();
                opened.origin.apply(&mut ev);
                Ok(crate::origin::Admitted {
                    message_id: opened.envelope.message_id.clone(),
                    event: ev,
                    origin: opened.origin.clone(),
                })
            } else {
                admit(&entry.policy, &opened, entry.types.as_ref(), now)
            };
            match admitted {
                Ok(a) => {
                    let Ok(to) = Address::parse(&stored.recipient) else {
                        continue;
                    };
                    let mut env = Envelope::new(a.message_id.clone(), &to, a.event.clone());
                    env.correlation.clone_from(&opened.envelope.correlation);
                    lock(&self.held).insert(key.clone());
                    let mb = self.mailbox(&stored.recipient);
                    if mb.push(env, key) {
                        self.record(InboundRecord::accepted(&stored.recipient, &a, now));
                        *report
                            .delivered
                            .entry(stored.recipient.clone())
                            .or_default() += 1;
                        woken.push(stored.recipient.clone());
                    } else {
                        report.duplicates += 1;
                    }
                }
                Err(refusal) => {
                    self.refuse(&stored.recipient, index, &stored, Some(&opened), refusal);
                    report.refused += 1;
                }
            }
        }
        woken.dedup();
        let handle = self.handle.read().ok().and_then(|h| h.clone());
        if let Some(h) = handle {
            for did in woken {
                if let Ok(a) = Address::parse(&did) {
                    h.notify(&a);
                }
            }
        }
    }

    fn pickup(&self) -> PickupReport {
        let mut report = PickupReport::default();
        for i in 0..self.bindings.len() {
            self.pickup_binding(i, &mut report);
        }
        report
    }
}

/// The relay mailboxes of a node: pickup bindings and the mailbox of each local DID actor.
/// Register it with the node for the `did` scheme
/// ([`vertep_runtime::NodeBuilder::mailboxes_for_scheme`]).
#[derive(Clone)]
// @tg: implements:: [[openspec:relay-mailboxes#DIDComm Message Pickup]]
// @tg: implements:: [[openspec:relay-mailboxes#Shared And Per-Actor Mediator Mailboxes]]
// @tg: implements:: [[openspec:relay-mailboxes#Activation On Delivery]]
// @tg: implements:: [[openspec:relay-mailboxes#At-Least-Once Delivery With Deduplication]]
// @tg: implements:: [[openspec:did-identity#Unauthenticated Event Marking]]
pub struct RelayInbox {
    pub(crate) core: Arc<InboxCore>,
}

impl fmt::Debug for RelayInbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayInbox")
            .field("bindings", &self.core.bindings)
            .finish_non_exhaustive()
    }
}

impl RelayInbox {
    pub(crate) fn new(
        bindings: Vec<PickupBinding>,
        identities: Arc<Identities>,
        resolver: Arc<dyn DidResolver>,
        clock: Arc<dyn Clock>,
        local: Arc<dyn MailboxFactory>,
        limit: usize,
        storage: Option<Arc<dyn Storage>>,
    ) -> Self {
        let core = Arc::new_cyclic(|me| InboxCore {
            bindings,
            mailboxes: Mutex::new(HashMap::new()),
            held: Mutex::new(HashSet::new()),
            identities,
            resolver,
            records: Mutex::new(Vec::new()),
            handle: RwLock::new(None),
            outbox: RwLock::new(Weak::new()),
            clock,
            local,
            limit: limit.max(1),
            storage,
            me: me.clone(),
        });
        RelayInbox { core }
    }

    pub(crate) fn attach(&self, handle: &NodeHandle, outbox: &Arc<OutboxCore>) {
        if let Ok(mut h) = self.core.handle.write() {
            *h = Some(handle.clone());
        }
        if let Ok(mut o) = self.core.outbox.write() {
            *o = Arc::downgrade(outbox);
        }
    }

    /// Polls every binding once (a message pickup batch) and places what it fetched.
    pub fn pickup(&self) -> PickupReport {
        self.core.pickup()
    }

    /// Opens a live session on every binding whose relay offers one: messages arriving later are
    /// picked up and delivered at once. Returns the names of the bindings now live.
    pub fn go_live(&self) -> Vec<String> {
        let mut live = Vec::new();
        for (i, b) in self.core.bindings.iter().enumerate() {
            let weak = Arc::downgrade(&self.core);
            let wake: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |_recipient: &str| {
                if let Some(core) = weak.upgrade() {
                    let mut r = PickupReport::default();
                    core.pickup_binding(i, &mut r);
                }
            });
            if b.source.live(&self.core.recipients(b), wake) {
                live.push(b.name.clone());
            }
        }
        live
    }

    /// The mailbox of the local DID actor `address`.
    pub fn mailbox(&self, address: &str) -> Arc<RelayMailbox> {
        self.core.mailbox(address)
    }

    /// Every inbound record (accepted and refused), in order.
    pub fn records(&self) -> Vec<InboundRecord> {
        lock(&self.core.records).clone()
    }

    /// The protocols of the bindings.
    pub fn transports(&self) -> Vec<TransportKind> {
        let mut v: Vec<_> = self
            .core
            .bindings
            .iter()
            .map(|b| b.protocol.kind())
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

impl MailboxFactory for RelayInbox {
    fn create(&self, address: &Address) -> Arc<dyn Mailbox> {
        self.core.mailbox(address.as_str())
    }

    /// After a restart: picks up what the relays held while the node was down, and names the
    /// actors that now have messages, so the node activates them.
    fn recover(&self) -> Vec<Address> {
        self.core.pickup();
        lock(&self.core.mailboxes)
            .iter()
            .filter(|(_, m)| m.holds_picked())
            .filter_map(|(a, _)| Address::parse(a).ok())
            .collect()
    }
}
