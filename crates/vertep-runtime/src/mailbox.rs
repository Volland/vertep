//! Mailboxes: where messages wait for an actor, independently of its activations.
//!
//! A mailbox keeps messages in arrival order, which preserves the order of every single sender.
//! [`Mailbox::receive`] hands out the oldest message that is not in flight and not deferred by
//! the receiver's active states, and marks it in flight; [`Mailbox::ack`] removes it once its
//! turn is durable; [`Mailbox::release`] puts it back when the turn failed, so it is delivered
//! again. Deferred messages are skipped, never removed, so they keep their place.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use oxrdf::{Literal, Quad, Term};
use serde::{Deserialize, Serialize};
use vertep_core::datamodel::Event;
use vertep_core::kernel::descriptor_matches;

use crate::address::Address;
use crate::storage::{Storage, select, text};
use crate::vocab::{self as vt, node};

/// A message on its way to an actor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// The deduplication key: stable across redeliveries and retries.
    pub message_id: String,
    /// The receiver's address.
    pub to: String,
    /// The event, with origin and origin type set by the sender's transport.
    pub event: Event,
    /// A request's correlation id, or the correlation id a reply answers.
    pub correlation: Option<String>,
    /// The timer whose firing this message is.
    pub timer: Option<String>,
}

impl Envelope {
    /// A message `event` to `to` with message id `message_id`.
    pub fn new(message_id: impl Into<String>, to: &Address, event: Event) -> Self {
        Envelope {
            message_id: message_id.into(),
            to: to.as_str().to_owned(),
            event,
            correlation: None,
            timer: None,
        }
    }
}

/// A mailbox failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxError(pub String);

impl core::fmt::Display for MailboxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "mailbox error: {}", self.0)
    }
}

impl std::error::Error for MailboxError {}

/// True when an event named `name` matches one of the `vt:defer` descriptors.
pub fn is_deferred(deferred: &[String], name: &str) -> bool {
    deferred.iter().any(|d| descriptor_matches(d, name))
}

/// The mailbox contract: enqueue, ordered receive with deferral, and acknowledge.
// @tg: implements:: [[openspec:messaging#Pluggable Mailboxes]]
// @tg: implements:: [[openspec:messaging#Mailbox Ordering]]
pub trait Mailbox: Send + Sync {
    /// Appends a message.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] when the message cannot be stored.
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError>;

    /// The oldest message that is neither in flight nor deferred (its event name matches no
    /// descriptor in `deferred`), now marked in flight.
    fn receive(&self, deferred: &[String]) -> Option<Envelope>;

    /// Removes the in-flight message `message_id` (its turn is durable).
    ///
    /// # Errors
    ///
    /// [`MailboxError`] when the removal cannot be stored.
    fn ack(&self, message_id: &str) -> Result<(), MailboxError>;

    /// Puts the in-flight message `message_id` back, to be received again.
    fn release(&self, message_id: &str);

    /// Puts every in-flight message back.
    fn release_all(&self);

    /// Every message not yet acknowledged, in arrival order.
    fn pending(&self) -> Vec<Envelope>;

    /// True when [`Mailbox::receive`] would return a message.
    fn has_deliverable(&self, deferred: &[String]) -> bool {
        self.pending_deliverable(deferred) > 0
    }

    /// The number of messages that are neither in flight nor deferred.
    fn pending_deliverable(&self, deferred: &[String]) -> usize;

    /// The number of messages not yet acknowledged.
    fn len(&self) -> usize {
        self.pending().len()
    }

    /// True when every message is acknowledged.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct Row {
    env: Envelope,
    seq: u64,
    in_flight: bool,
}

#[derive(Default)]
struct Rows {
    rows: VecDeque<Row>,
    next: u64,
}

impl Rows {
    fn receive(&mut self, deferred: &[String]) -> Option<Envelope> {
        let r = self
            .rows
            .iter_mut()
            .find(|r| !r.in_flight && !is_deferred(deferred, &r.env.event.name))?;
        r.in_flight = true;
        Some(r.env.clone())
    }

    fn remove(&mut self, id: &str) -> Option<Row> {
        let i = self
            .rows
            .iter()
            .position(|r| r.in_flight && r.env.message_id == id)
            .or_else(|| self.rows.iter().position(|r| r.env.message_id == id))?;
        self.rows.remove(i)
    }

    fn release(&mut self, id: &str) {
        for r in &mut self.rows {
            if r.env.message_id == id {
                r.in_flight = false;
            }
        }
    }

    fn deliverable(&self, deferred: &[String]) -> usize {
        self.rows
            .iter()
            .filter(|r| !r.in_flight && !is_deferred(deferred, &r.env.event.name))
            .count()
    }
}

fn lock(m: &Mutex<Rows>) -> MutexGuard<'_, Rows> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An in-memory mailbox; its messages are lost with the process.
#[derive(Default)]
pub struct MemoryMailbox {
    rows: Mutex<Rows>,
}

impl std::fmt::Debug for MemoryMailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryMailbox")
            .field("len", &self.len())
            .finish()
    }
}

impl Mailbox for MemoryMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        let mut g = lock(&self.rows);
        let seq = g.next;
        g.next += 1;
        g.rows.push_back(Row {
            env,
            seq,
            in_flight: false,
        });
        Ok(())
    }

    fn receive(&self, deferred: &[String]) -> Option<Envelope> {
        lock(&self.rows).receive(deferred)
    }

    fn ack(&self, message_id: &str) -> Result<(), MailboxError> {
        lock(&self.rows).remove(message_id);
        Ok(())
    }

    fn release(&self, message_id: &str) {
        lock(&self.rows).release(message_id);
    }

    fn release_all(&self) {
        for r in &mut lock(&self.rows).rows {
            r.in_flight = false;
        }
    }

    fn pending(&self) -> Vec<Envelope> {
        lock(&self.rows)
            .rows
            .iter()
            .map(|r| r.env.clone())
            .collect()
    }

    fn pending_deliverable(&self, deferred: &[String]) -> usize {
        lock(&self.rows).deliverable(deferred)
    }
}

/// A mailbox kept in the actor's graph store, in graph `<address>/mailbox`: it survives
/// restarts, and its messages are delivered again in their original order.
// @tg: implements:: [[openspec:messaging#Mailboxes Outlive Activations]]
pub struct StoreMailbox {
    address: Address,
    storage: Arc<dyn Storage>,
    rows: Mutex<Rows>,
}

impl std::fmt::Debug for StoreMailbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreMailbox")
            .field("address", &self.address)
            .field("len", &self.len())
            .finish()
    }
}

impl StoreMailbox {
    /// Opens the mailbox of `address`, loading the messages stored for it.
    pub fn open(address: Address, storage: Arc<dyn Storage>) -> Self {
        let mut rows = Rows::default();
        let g = address.mailbox_graph();
        let q = format!(
            "SELECT ?seq ?env WHERE {{ GRAPH {g} {{ ?m <{seq}> ?seq ; <{env}> ?env }} }}",
            seq = vt::SEQUENCE,
            env = vt::ENVELOPE
        );
        if let Ok(sol) = select(storage.graph(address.node()), &q) {
            let mut found: Vec<(u64, Envelope)> = Vec::new();
            for i in 0..sol.len() {
                let seq = sol
                    .get(i, "seq")
                    .and_then(text)
                    .and_then(|s| s.parse::<u64>().ok());
                let env = sol
                    .get(i, "env")
                    .and_then(text)
                    .and_then(|s| serde_json::from_str::<Envelope>(&s).ok());
                if let (Some(seq), Some(env)) = (seq, env) {
                    found.push((seq, env));
                }
            }
            // Rows written by other nodes sharing the store may repeat a sequence number; the
            // message id orders those deterministically.
            found.sort_by(|(s, a), (t, b)| s.cmp(t).then_with(|| a.message_id.cmp(&b.message_id)));
            for (seq, env) in found {
                rows.next = rows.next.max(seq + 1);
                rows.rows.push_back(Row {
                    env,
                    seq,
                    in_flight: false,
                });
            }
        }
        StoreMailbox {
            address,
            storage,
            rows: Mutex::new(rows),
        }
    }

    fn quads(&self, seq: u64, env: &Envelope) -> Vec<Quad> {
        let g = self.address.mailbox_graph();
        let json = serde_json::to_string(env).unwrap_or_default();
        let m = self.address.mailbox_row_iri(seq, &json);
        let q = |p: &str, o: Term| Quad::new(m.clone(), node(p), o, g.clone());
        vec![
            q(vt::RDF_TYPE, node(vt::MESSAGE).into()),
            q(vt::RECIPIENT, self.address.node().clone().into()),
            q(
                vt::SEQUENCE,
                Literal::new_typed_literal(seq.to_string(), oxrdf::vocab::xsd::INTEGER).into(),
            ),
            q(
                vt::MESSAGE_ID,
                Literal::new_simple_literal(&env.message_id).into(),
            ),
            q(vt::ENVELOPE, Literal::new_simple_literal(json).into()),
        ]
    }
}

impl Mailbox for StoreMailbox {
    fn enqueue(&self, env: Envelope) -> Result<(), MailboxError> {
        let mut g = lock(&self.rows);
        let seq = g.next;
        self.storage
            .graph(self.address.node())
            .insert_quads(&self.quads(seq, &env))
            .map_err(|e| MailboxError(e.to_string()))?;
        g.next += 1;
        g.rows.push_back(Row {
            env,
            seq,
            in_flight: false,
        });
        Ok(())
    }

    fn receive(&self, deferred: &[String]) -> Option<Envelope> {
        lock(&self.rows).receive(deferred)
    }

    fn ack(&self, message_id: &str) -> Result<(), MailboxError> {
        let mut g = lock(&self.rows);
        if let Some(r) = g.remove(message_id) {
            self.storage
                .graph(self.address.node())
                .remove_quads(&self.quads(r.seq, &r.env))
                .map_err(|e| MailboxError(e.to_string()))?;
        }
        Ok(())
    }

    fn release(&self, message_id: &str) {
        lock(&self.rows).release(message_id);
    }

    fn release_all(&self) {
        for r in &mut lock(&self.rows).rows {
            r.in_flight = false;
        }
    }

    fn pending(&self) -> Vec<Envelope> {
        lock(&self.rows)
            .rows
            .iter()
            .map(|r| r.env.clone())
            .collect()
    }

    fn pending_deliverable(&self, deferred: &[String]) -> usize {
        lock(&self.rows).deliverable(deferred)
    }
}

/// Creates the mailbox of an address.
pub trait MailboxFactory: Send + Sync {
    /// The mailbox of `address`.
    fn create(&self, address: &Address) -> Arc<dyn Mailbox>;
    /// The addresses that have stored messages, so that a restarted node delivers them.
    fn recover(&self) -> Vec<Address> {
        Vec::new()
    }
}

/// In-memory mailboxes.
#[derive(Clone, Copy, Debug, Default)]
pub struct MemoryMailboxes;

impl MailboxFactory for MemoryMailboxes {
    fn create(&self, _address: &Address) -> Arc<dyn Mailbox> {
        Arc::new(MemoryMailbox::default())
    }
}

/// Store-backed mailboxes on a node's storage.
#[derive(Clone)]
pub struct StoreMailboxes {
    storage: Arc<dyn Storage>,
}

impl std::fmt::Debug for StoreMailboxes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreMailboxes").finish_non_exhaustive()
    }
}

impl StoreMailboxes {
    /// Mailboxes stored in `storage`.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        StoreMailboxes { storage }
    }
}

impl MailboxFactory for StoreMailboxes {
    fn create(&self, address: &Address) -> Arc<dyn Mailbox> {
        Arc::new(StoreMailbox::open(address.clone(), self.storage.clone()))
    }

    fn recover(&self) -> Vec<Address> {
        let q = format!(
            "SELECT DISTINCT ?a WHERE {{ GRAPH ?g {{ ?m a <{msg}> ; <{rcpt}> ?a }} }}",
            msg = vt::MESSAGE,
            rcpt = vt::RECIPIENT
        );
        let mut out = Vec::new();
        for g in self.storage.graphs() {
            if let Ok(sol) = select(g, &q) {
                for t in sol.values("a") {
                    if let Some(a) = text(&t).and_then(|s| Address::parse(&s).ok()) {
                        out.push(a);
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Which addresses a [`RoutedMailboxes`] rule applies to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressMatch {
    /// Every address of one scheme (`vt`, `did`, `https`).
    Scheme(String),
    /// Every address that starts with the prefix.
    Prefix(String),
    /// Exactly one address.
    Exact(String),
}

impl AddressMatch {
    /// True when `a` is selected.
    pub fn matches(&self, a: &Address) -> bool {
        match self {
            AddressMatch::Scheme(s) => a.scheme() == s,
            AddressMatch::Prefix(p) => a.as_str().starts_with(p.as_str()),
            AddressMatch::Exact(e) => a.as_str() == e,
        }
    }
}

/// Where a mailbox keeps its messages, chosen per route.
///
/// Every variant becomes a [`MailboxFactory`]; `Custom` takes any other, such as a relay inbox
/// whose pickup source is a remote TSP mediator.
#[derive(Clone)]
pub enum MailboxBackend {
    /// In memory: lost with the process.
    Memory,
    /// Rows in a graph store, which may be a different store from the node's journal: a
    /// [`vertep_store::MemoryStore`], a tiramemsu shard file or any other [`Storage`].
    Store(Arc<dyn Storage>),
    /// Any other factory, for example `RelayInbox` (a mediator holds the messages).
    Custom(Arc<dyn MailboxFactory>),
}

impl std::fmt::Debug for MailboxBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MailboxBackend::Memory => f.write_str("Memory"),
            MailboxBackend::Store(_) => f.write_str("Store"),
            MailboxBackend::Custom(_) => f.write_str("Custom"),
        }
    }
}

impl MailboxBackend {
    /// Store-backed mailboxes in a tiramemsu directory of `shard_count` files, opened here
    /// (one writer per file, so the directory must not be open elsewhere in the process).
    ///
    /// # Errors
    ///
    /// The store's open errors.
    #[cfg(feature = "tiramemsu")]
    pub fn tiramemsu(
        dir: impl AsRef<std::path::Path>,
        shard_count: usize,
    ) -> Result<Self, vertep_store::StoreError> {
        let backend = vertep_tiramemsu::TiramemsuBackend::open(dir, shard_count)?;
        Ok(MailboxBackend::Store(Arc::new(backend)))
    }

    /// The factory this backend stands for.
    pub fn factory(&self) -> Arc<dyn MailboxFactory> {
        match self {
            MailboxBackend::Memory => Arc::new(MemoryMailboxes),
            MailboxBackend::Store(s) => Arc::new(StoreMailboxes::new(s.clone())),
            MailboxBackend::Custom(f) => f.clone(),
        }
    }
}

/// A [`MailboxFactory`] that picks the backend per address: the first matching rule wins, and
/// the default serves the rest. A node's mailboxes, the federation's local mailboxes and any
/// route of another router can all be a `RoutedMailboxes`.
///
/// ```ignore
/// let boxes = RoutedMailboxes::new(MailboxBackend::Memory)
///     .route(AddressMatch::Scheme("did".into()), MailboxBackend::Custom(inbox))
///     .route(AddressMatch::Prefix("vt://orders/".into()), MailboxBackend::tiramemsu(dir, 4)?);
/// ```
// @tg: implements:: [[openspec:messaging#Pluggable Mailboxes]]
// @tg: implements:: [[decisions#Mailbox backend is chosen per address]]
#[derive(Clone)]
pub struct RoutedMailboxes {
    rules: Vec<(AddressMatch, Arc<dyn MailboxFactory>)>,
    default: Arc<dyn MailboxFactory>,
}

impl std::fmt::Debug for RoutedMailboxes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutedMailboxes")
            .field("rules", &self.rules.len())
            .finish_non_exhaustive()
    }
}

impl RoutedMailboxes {
    /// Routes everything to `default` until rules are added.
    pub fn new(default: MailboxBackend) -> Self {
        RoutedMailboxes {
            rules: Vec::new(),
            default: default.factory(),
        }
    }

    /// Addresses selected by `when` use `backend`. Rules are tried in the order added.
    #[must_use]
    pub fn route(mut self, when: AddressMatch, backend: MailboxBackend) -> Self {
        self.rules.push((when, backend.factory()));
        self
    }

    fn factory_for(&self, a: &Address) -> &Arc<dyn MailboxFactory> {
        self.rules
            .iter()
            .find(|(m, _)| m.matches(a))
            .map_or(&self.default, |(_, f)| f)
    }
}

impl MailboxFactory for RoutedMailboxes {
    fn create(&self, address: &Address) -> Arc<dyn Mailbox> {
        self.factory_for(address).create(address)
    }

    /// The addresses every backend has messages for.
    fn recover(&self) -> Vec<Address> {
        let mut out: Vec<Address> = self.default.recover();
        for (_, f) in &self.rules {
            out.extend(f.recover());
        }
        out.sort();
        out.dedup();
        out
    }
}

/// A mailbox backend as written in a configuration file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendSpec {
    /// In memory.
    Memory,
    /// Rows in a tiramemsu directory of `shards` files (needs the `tiramemsu` feature).
    Tiramemsu {
        /// The directory, created when missing.
        dir: String,
        /// The number of shard files.
        shards: usize,
    },
    /// A factory the application registered under this name, such as a relay inbox whose
    /// pickup source is a TSP mediator.
    Named(String),
}

/// One routing rule in a configuration file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteSpec {
    /// The addresses it applies to.
    #[serde(rename = "match")]
    pub when: MatchSpec,
    /// Where their mailboxes live.
    pub backend: BackendSpec,
}

/// Address selection as written in a configuration file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchSpec {
    /// Every address of the scheme.
    Scheme(String),
    /// Every address with the prefix.
    Prefix(String),
    /// One address.
    Exact(String),
}

/// The mailbox configuration of a node: a default backend and ordered routes.
///
/// ```json
/// { "default": "memory",
///   "routes": [
///     { "match": { "scheme": "did" }, "backend": { "named": "tsp" } },
///     { "match": { "prefix": "vt://orders/" },
///       "backend": { "tiramemsu": { "dir": "data/orders-mailboxes", "shards": 4 } } } ] }
/// ```
// @tg: implements:: [[openspec:messaging#Pluggable Mailboxes]]
// @tg: implements:: [[decisions#Mailbox backend is chosen per address]]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxConfig {
    /// The backend of every address no route selects.
    #[serde(default = "MailboxConfig::memory")]
    pub default: BackendSpec,
    /// Routes, tried in order.
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
}

impl Default for MailboxConfig {
    fn default() -> Self {
        MailboxConfig {
            default: BackendSpec::Memory,
            routes: Vec::new(),
        }
    }
}

impl MailboxConfig {
    fn memory() -> BackendSpec {
        BackendSpec::Memory
    }

    /// Parses a JSON configuration.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] when the text is not a valid configuration.
    pub fn from_json(text: &str) -> Result<Self, MailboxError> {
        serde_json::from_str(text).map_err(|e| MailboxError(format!("mailbox config: {e}")))
    }

    /// Checks the configuration without opening anything: every tiramemsu backend has a
    /// directory and at least one shard, every name and match is non-empty. Returns the names
    /// the application must register, in order of first use.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] naming the first problem.
    pub fn validate(&self) -> Result<Vec<String>, MailboxError> {
        let mut names: Vec<String> = Vec::new();
        let specs = std::iter::once(("default".to_owned(), &self.default)).chain(
            self.routes
                .iter()
                .enumerate()
                .map(|(i, r)| (format!("route {}", i + 1), &r.backend)),
        );
        for (at, spec) in specs {
            let bad = |why: &str| MailboxError(format!("mailbox config: {at}: {why}"));
            match spec {
                BackendSpec::Memory => {}
                BackendSpec::Tiramemsu { dir, shards } => {
                    if dir.is_empty() {
                        return Err(bad("tiramemsu needs a dir"));
                    }
                    if *shards == 0 {
                        return Err(bad("tiramemsu needs at least one shard"));
                    }
                }
                BackendSpec::Named(n) => {
                    if n.is_empty() {
                        return Err(bad("a named backend needs a name"));
                    }
                    if !names.contains(n) {
                        names.push(n.clone());
                    }
                }
            }
        }
        for (i, r) in self.routes.iter().enumerate() {
            let (MatchSpec::Scheme(t) | MatchSpec::Prefix(t) | MatchSpec::Exact(t)) = &r.when;
            if t.is_empty() {
                return Err(MailboxError(format!(
                    "mailbox config: route {}: the match is empty",
                    i + 1
                )));
            }
        }
        Ok(names)
    }

    /// Builds the router. `named` holds the factories that `named` backends refer to.
    ///
    /// # Errors
    ///
    /// [`MailboxError`] for a name that is not in `named`, for a tiramemsu backend when the
    /// feature is off, and for a store that cannot be opened.
    pub fn build(
        &self,
        named: &std::collections::BTreeMap<String, Arc<dyn MailboxFactory>>,
    ) -> Result<RoutedMailboxes, MailboxError> {
        self.validate()?;
        let backend = |spec: &BackendSpec| -> Result<MailboxBackend, MailboxError> {
            match spec {
                BackendSpec::Memory => Ok(MailboxBackend::Memory),
                BackendSpec::Named(n) => named
                    .get(n)
                    .cloned()
                    .map(MailboxBackend::Custom)
                    .ok_or_else(|| MailboxError(format!("mailbox config: no backend named {n}"))),
                #[cfg(feature = "tiramemsu")]
                BackendSpec::Tiramemsu { dir, shards } => {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| MailboxError(format!("mailbox config: {dir}: {e}")))?;
                    MailboxBackend::tiramemsu(dir, *shards)
                        .map_err(|e| MailboxError(format!("mailbox config: {dir}: {e}")))
                }
                #[cfg(not(feature = "tiramemsu"))]
                BackendSpec::Tiramemsu { .. } => Err(MailboxError(
                    "mailbox config: tiramemsu backend needs the `tiramemsu` feature".into(),
                )),
            }
        };
        let mut routed = RoutedMailboxes::new(backend(&self.default)?);
        for r in &self.routes {
            let when = match &r.when {
                MatchSpec::Scheme(s) => AddressMatch::Scheme(s.clone()),
                MatchSpec::Prefix(p) => AddressMatch::Prefix(p.clone()),
                MatchSpec::Exact(e) => AddressMatch::Exact(e.clone()),
            };
            routed = routed.route(when, backend(&r.backend)?);
        }
        Ok(routed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vertep_core::datamodel::EventType;

    // @tg: verifies:: [[openspec:messaging#Pluggable Mailboxes#Mailbox routes from configuration]] {level: unit}
    #[test]
    fn config_builds_routes_and_rejects_unknown_names() {
        let json = r#"{"routes":[
            {"match":{"prefix":"urn:t:"},"backend":{"named":"relay"}},
            {"match":{"scheme":"vt"},"backend":"memory"}]}"#;
        let cfg = MailboxConfig::from_json(json).unwrap();
        assert_eq!(cfg.default, BackendSpec::Memory);
        assert_eq!(cfg.routes.len(), 2);
        let err = cfg.build(&Default::default()).unwrap_err();
        assert!(err.0.contains("no backend named relay"), "{err}");
        let relay = Arc::new(vertep_store::MemoryStore::new());
        let mut named: std::collections::BTreeMap<String, Arc<dyn MailboxFactory>> =
            Default::default();
        named.insert("relay".into(), Arc::new(StoreMailboxes::new(relay.clone())));
        let boxes = cfg.build(&named).unwrap();
        boxes
            .create(&Address::parse("urn:t:a").unwrap())
            .enqueue(env("m1", "x"))
            .unwrap();
        assert_eq!(boxes.recover().len(), 1);
        assert!(MailboxConfig::from_json("{\"default\": 3}").is_err());
    }

    #[test]
    fn validate_names_what_to_register_and_rejects_bad_specs() {
        let ok = MailboxConfig::from_json(
            r#"{"routes":[{"match":{"scheme":"did"},"backend":{"named":"tsp"}},
                          {"match":{"scheme":"vt"},"backend":{"named":"tsp"}}]}"#,
        )
        .unwrap();
        assert_eq!(ok.validate().unwrap(), vec!["tsp".to_owned()]);
        for bad in [
            r#"{"default":{"tiramemsu":{"dir":"d","shards":0}}}"#,
            r#"{"default":{"tiramemsu":{"dir":"","shards":2}}}"#,
            r#"{"routes":[{"match":{"prefix":""},"backend":"memory"}]}"#,
            r#"{"routes":[{"match":{"scheme":"x"},"backend":{"named":""}}]}"#,
        ] {
            let cfg = MailboxConfig::from_json(bad).unwrap();
            assert!(cfg.validate().is_err(), "{bad}");
            assert!(cfg.build(&Default::default()).is_err(), "{bad}");
        }
    }

    #[test]
    fn routes_pick_backend_per_address_first_match_wins() {
        let durable = Arc::new(vertep_store::MemoryStore::new());
        let boxes = RoutedMailboxes::new(MailboxBackend::Memory)
            .route(
                AddressMatch::Exact("urn:t:special".into()),
                MailboxBackend::Memory,
            )
            .route(
                AddressMatch::Prefix("urn:t:".into()),
                MailboxBackend::Store(durable.clone()),
            );
        let kept = Address::parse("urn:t:a").unwrap();
        let lost = Address::parse("urn:t:special").unwrap();
        boxes.create(&kept).enqueue(env("m1", "x")).unwrap();
        boxes.create(&lost).enqueue(env("m2", "x")).unwrap();
        // A new router over the same store: the prefix route's message is recovered, the
        // exact route's (in memory) is not.
        let again = RoutedMailboxes::new(MailboxBackend::Memory).route(
            AddressMatch::Prefix("urn:t:".into()),
            MailboxBackend::Store(durable),
        );
        assert_eq!(again.recover(), vec![kept.clone()]);
        assert_eq!(again.create(&kept).pending().len(), 1);
        assert!(again.create(&lost).pending().is_empty());
    }

    fn env(id: &str, name: &str) -> Envelope {
        Envelope::new(
            id,
            &Address::parse("urn:t:a").unwrap(),
            Event::named(name, EventType::External),
        )
    }

    #[test]
    fn receive_skips_deferred_and_in_flight_and_keeps_order() {
        let m = MemoryMailbox::default();
        m.enqueue(env("1", "order.cancel")).unwrap();
        m.enqueue(env("2", "ping")).unwrap();
        m.enqueue(env("3", "pong")).unwrap();
        let d = vec!["order.*".to_owned()];
        assert_eq!(m.receive(&d).unwrap().message_id, "2");
        assert_eq!(m.receive(&d).unwrap().message_id, "3");
        assert!(m.receive(&d).is_none());
        m.release("2");
        assert_eq!(m.receive(&[]).unwrap().message_id, "1");
        m.ack("1").unwrap();
        assert_eq!(m.receive(&[]).unwrap().message_id, "2");
        assert_eq!(m.len(), 2);
    }
}
