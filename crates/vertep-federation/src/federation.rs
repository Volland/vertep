//! Wiring federation into a runtime node: the local DID actors, the relay outbox and inbox, the
//! relay transports, and the identity graph.
//!
//! ```text
//! let fed = Federation::builder(resolver, network).protocol(didcomm).pickup(binding).build();
//! let node = Node::builder(storage, shell)
//!     .mailboxes_for_scheme("did", fed.mailboxes())   // local DID actors' relay mailboxes
//!     .transport(fed.transport(TransportKind::DidComm)) // <send type="didcomm">
//!     .build();
//! fed.attach(&node.handle());                       // did: resolver, wake-ups, error path
//! let did = fed.provision(&node.handle(), chart_type, key, &config, km)?;  // DID as address
//! loop { node.run_until_idle(); fed.pump(); }
//! ```
//!
//! Actors on `vt:` IRIs never touch any of this: their sends resolve through the cluster
//! directory and are delivered without DID resolution, keys or cryptography.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use oxrdf::{GraphName, NamedNode, Quad};
use vertep_runtime::mailbox::MemoryMailbox;
use vertep_runtime::{Address, Mailbox, MailboxFactory, MemoryMailboxes, NodeHandle, Storage};

use crate::directory::{DidDirectory, TransportKind};
use crate::grants::Grant;
use crate::identity::{ActorIdentity, IdentityConfig};
use crate::inbox::{PickupBinding, RelayInbox};
use crate::keys::{KeyError, KeyManager};
use crate::net::RelayNetwork;
use crate::ontology::{ProjectionError, project_actor, project_grants};
use crate::origin::{AnonymousPolicy, InboundPolicy, InboundRecord, PayloadTypes, Untyped};
use crate::outbox::{OutboundRecord, OutboxCore, RelayOutbox, RelayTransport, RetryPolicy};
use crate::relay::{LocalActor, RelayProtocol};
use crate::resolver::{CachingResolver, DidResolver};
use crate::time::{Clock, SystemClock, Timestamp};

/// A local DID actor with its inbound rules.
#[derive(Clone)]
pub struct ActorEntry {
    /// The identity and its key manager.
    pub actor: LocalActor,
    /// Anonymous-sender policy and inbound grants.
    pub policy: InboundPolicy,
    /// The receiving chart's declared payload types.
    pub types: Arc<dyn PayloadTypes + Send + Sync>,
}

impl fmt::Debug for ActorEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActorEntry")
            .field("actor", &self.actor)
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// The DID actors hosted on this node.
#[derive(Debug, Default)]
pub struct Identities {
    actors: RwLock<BTreeMap<String, ActorEntry>>,
}

impl Identities {
    /// The entry of the local actor `did`.
    pub fn entry(&self, did: &str) -> Option<ActorEntry> {
        self.actors.read().ok().and_then(|a| a.get(did).cloned())
    }

    /// The local actor `did`.
    pub fn actor(&self, did: &str) -> Option<LocalActor> {
        self.entry(did).map(|e| e.actor)
    }

    /// True when `did` is hosted here.
    pub fn is_local(&self, did: &str) -> bool {
        self.actors.read().is_ok_and(|a| a.contains_key(did))
    }

    /// Every local DID.
    pub fn dids(&self) -> Vec<String> {
        self.actors
            .read()
            .map(|a| a.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn insert(&self, e: ActorEntry) {
        if let Ok(mut a) = self.actors.write() {
            a.insert(e.actor.identity.did.clone(), e);
        }
    }

    fn update(&self, did: &str, f: impl FnOnce(&mut ActorEntry)) -> bool {
        self.actors
            .write()
            .ok()
            .and_then(|mut a| a.get_mut(did).map(f))
            .is_some()
    }
}

/// The runtime shell's clock as a federation [`Clock`], so grant expiry, cache lifetimes and
/// retries follow the node's (possibly virtual) time.
#[derive(Clone)]
pub struct RuntimeClock(pub Arc<dyn vertep_runtime::Clock>);

impl fmt::Debug for RuntimeClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RuntimeClock")
    }
}

impl Clock for RuntimeClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.now_ms())
    }
}

/// Builds a [`Federation`].
pub struct FederationBuilder {
    resolver: Arc<dyn DidResolver>,
    net: Arc<dyn RelayNetwork>,
    clock: Arc<dyn Clock>,
    ttl: Duration,
    cache_entries: usize,
    protocols: BTreeMap<TransportKind, Arc<dyn RelayProtocol>>,
    preference: Vec<TransportKind>,
    retry: RetryPolicy,
    bindings: Vec<PickupBinding>,
    storage: Option<Arc<dyn Storage>>,
    local: Arc<dyn MailboxFactory>,
    limit: usize,
}

impl fmt::Debug for FederationBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FederationBuilder")
            .field("preference", &self.preference)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl FederationBuilder {
    /// The clock (default: the system clock).
    pub fn clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// How long resolved DID documents are cached (default 5 minutes) and how many.
    pub fn cache(mut self, ttl: Duration, max_entries: usize) -> Self {
        self.ttl = ttl;
        self.cache_entries = max_entries;
        self
    }

    /// Adds a relay protocol.
    pub fn protocol(mut self, p: Arc<dyn RelayProtocol>) -> Self {
        self.protocols.insert(p.kind(), p);
        self
    }

    /// The transport preference (default: TSP, then DIDComm).
    pub fn preference(mut self, p: Vec<TransportKind>) -> Self {
        self.preference = p;
        self
    }

    /// The retry policy of cross-trust sends.
    pub fn retry(mut self, r: RetryPolicy) -> Self {
        self.retry = r;
        self
    }

    /// Adds a pickup binding (a mediator, intermediary or inbound queue to fetch from).
    pub fn pickup(mut self, b: PickupBinding) -> Self {
        self.bindings.push(b);
        self
    }

    /// The node's storage: the relay outbox becomes store-backed (it survives restarts) and
    /// identity graphs are written there.
    pub fn storage(mut self, s: Arc<dyn Storage>) -> Self {
        self.storage = Some(s);
        self
    }

    /// The mailboxes for messages from senders on this node (default: in memory).
    pub fn local_mailboxes(mut self, f: Arc<dyn MailboxFactory>) -> Self {
        self.local = f;
        self
    }

    /// The most messages one fetch asks a relay for (default 100).
    pub fn batch(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Builds the federation layer.
    pub fn build(self) -> Federation {
        let identities = Arc::new(Identities::default());
        let resolver = Arc::new(CachingResolver::new(
            self.resolver,
            self.ttl,
            self.cache_entries,
            self.clock.clone(),
        ));
        let directory = DidDirectory::new(resolver.clone() as Arc<dyn DidResolver>)
            .with_preference(self.preference);
        let queue: Arc<dyn Mailbox> = match &self.storage {
            Some(s) => Arc::new(vertep_runtime::mailbox::StoreMailbox::open(
                Address::parse(OUTBOX_ADDRESS).expect("the outbox address is an IRI"),
                s.clone(),
            )),
            None => Arc::new(MemoryMailbox::default()),
        };
        let outbox = Arc::new(OutboxCore::new(
            queue,
            directory.clone(),
            resolver.clone(),
            self.protocols.clone(),
            self.net.clone(),
            identities.clone(),
            self.retry,
            self.clock.clone(),
        ));
        let inbox = RelayInbox::new(
            self.bindings,
            identities.clone(),
            resolver.clone() as Arc<dyn DidResolver>,
            self.clock.clone(),
            self.local,
            self.limit,
            self.storage.clone(),
        );
        Federation {
            identities,
            resolver,
            directory,
            outbox: RelayOutbox { core: outbox },
            inbox,
            clock: self.clock,
            storage: self.storage,
        }
    }
}

/// The address of the store-backed relay outbox queue.
pub const OUTBOX_ADDRESS: &str = "urn:vertep:federation:relay-outbox";

/// The federation layer of one node.
pub struct Federation {
    identities: Arc<Identities>,
    resolver: Arc<CachingResolver>,
    directory: DidDirectory,
    outbox: RelayOutbox,
    inbox: RelayInbox,
    clock: Arc<dyn Clock>,
    storage: Option<Arc<dyn Storage>>,
}

impl fmt::Debug for Federation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Federation")
            .field("actors", &self.identities.dids())
            .finish_non_exhaustive()
    }
}

/// Why provisioning a DID actor failed.
#[derive(Debug)]
pub enum ProvisionError {
    /// The key manager failed.
    Key(KeyError),
    /// The DID is not a valid address IRI.
    Address(String),
}

impl fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProvisionError::Key(e) => e.fmt(f),
            ProvisionError::Address(a) => write!(f, "not an address: {a}"),
        }
    }
}

impl std::error::Error for ProvisionError {}

impl Federation {
    /// A builder over `resolver` (usually a [`crate::ResolverRegistry`]) and `net`.
    pub fn builder(
        resolver: Arc<dyn DidResolver>,
        net: Arc<dyn RelayNetwork>,
    ) -> FederationBuilder {
        FederationBuilder {
            resolver,
            net,
            clock: Arc::new(SystemClock),
            ttl: Duration::from_secs(300),
            cache_entries: 1024,
            protocols: BTreeMap::new(),
            preference: vec![TransportKind::Tsp, TransportKind::DidComm],
            retry: RetryPolicy::default(),
            bindings: Vec::new(),
            storage: None,
            local: Arc::new(MemoryMailboxes),
            limit: 100,
        }
    }

    /// The mailbox factory for the `did` scheme.
    pub fn mailboxes(&self) -> Arc<dyn MailboxFactory> {
        Arc::new(self.inbox.clone())
    }

    /// The SCXML Event I/O Processor of a relay protocol, to register on the node.
    pub fn transport(&self, kind: TransportKind) -> Arc<dyn vertep_runtime::Transport> {
        Arc::new(RelayTransport::new(self.outbox.core.clone(), kind))
    }

    /// Connects to a built node: registers the `did:` resolver, and lets the outbox raise
    /// `error.communication` and the inbox wake actors.
    pub fn attach(&self, handle: &NodeHandle) {
        handle.register_resolver("did", Arc::new(self.outbox.clone()));
        if let Ok(mut h) = self.outbox.core.handle.write() {
            *h = Some(handle.clone());
        }
        self.inbox.attach(handle, &self.outbox.core);
    }

    /// Hosts the DID actor `actor` here with its inbound rules.
    pub fn host(
        &self,
        actor: LocalActor,
        anonymous: AnonymousPolicy,
        types: Arc<dyn PayloadTypes + Send + Sync>,
    ) {
        let did = actor.identity.did.clone();
        self.identities.insert(ActorEntry {
            actor,
            policy: InboundPolicy {
                actor: did,
                anonymous,
                grants: Default::default(),
            },
            types,
        });
    }

    /// Provisions a new instance of `chart_type` with key `key` whose address is a fresh DID
    /// (per `config`), hosts it here, binds the DID on the node and publishes its identity
    /// graph. Returns the identity; its DID is the actor's address.
    ///
    /// # Errors
    ///
    /// [`ProvisionError`] when the keys cannot be created.
    // @tg: implements:: [[openspec:did-identity#Opt-In DID Identity Per Actor]]
    pub fn provision(
        &self,
        handle: &NodeHandle,
        chart_type: &str,
        key: &str,
        config: &IdentityConfig,
        km: Arc<dyn KeyManager>,
    ) -> Result<ActorIdentity, ProvisionError> {
        let identity =
            ActorIdentity::provision(km.as_ref(), config).map_err(ProvisionError::Key)?;
        let address = Address::parse(&identity.did)
            .map_err(|_| ProvisionError::Address(identity.did.clone()))?;
        handle.bind_address(&address, chart_type, key);
        self.host(
            LocalActor {
                identity: identity.clone(),
                km,
            },
            AnonymousPolicy::Refuse,
            Arc::new(Untyped),
        );
        let _ = self.publish_identity(&identity.did);
        Ok(identity)
    }

    /// Sets the anonymous-sender policy of the local actor `did`.
    pub fn set_anonymous(&self, did: &str, policy: AnonymousPolicy) -> bool {
        self.identities.update(did, |e| e.policy.anonymous = policy)
    }

    /// Sets the declared payload types the local actor `did` checks received events against.
    pub fn set_payload_types(&self, did: &str, types: Arc<dyn PayloadTypes + Send + Sync>) -> bool {
        self.identities.update(did, |e| e.types = types)
    }

    /// Grants `grant.grantee` the right to send `grant.events` to the local actor
    /// `grant.target`, and republishes its identity graph.
    pub fn grant_inbound(&self, grant: Grant) -> bool {
        let target = grant.target.clone();
        let ok = self
            .identities
            .update(&target, |e| e.policy.grants.add(grant));
        if ok {
            let _ = self.publish_identity(&target);
        }
        ok
    }

    /// Verifies a grant credential presented to the local actor `receiver` (issuer in
    /// `trusted_issuers`, proof, subject, target and validity) and, when it holds, adds the grant
    /// it carries to the actor's inbound grants.
    ///
    /// # Errors
    ///
    /// [`crate::CredentialError`] naming the first check that failed; nothing is granted.
    // @tg: implements:: [[openspec:did-identity#Credential-Based Grants]]
    pub fn accept_credential(
        &self,
        receiver: &str,
        credential: &serde_json::Value,
        trusted_issuers: &[String],
    ) -> Result<Grant, crate::CredentialError> {
        let grant = crate::verify_grant_credential(
            credential,
            receiver,
            trusted_issuers,
            self.resolver.as_ref(),
            self.clock.now(),
        )?;
        self.grant_inbound(grant.clone());
        Ok(grant)
    }

    /// Lets the actor `grant.grantee` (a DID or `vt:` address) send `grant.events` to the
    /// cross-trust target `grant.target`.
    pub fn grant_outbound(&self, grant: Grant) {
        if let Ok(mut g) = self.outbox.core.grants.write() {
            g.add(grant);
        }
    }

    /// Replaces the identity of a local actor after a key rotation (same DID), and republishes
    /// its identity graph; the previous graph state stays in the store's history.
    pub fn update_identity(&self, identity: ActorIdentity) -> bool {
        let did = identity.did.clone();
        let ok = self
            .identities
            .update(&did, |e| e.actor.identity = identity);
        if ok {
            let _ = self.publish_identity(&did);
        }
        ok
    }

    /// The graph IRI holding the identity of `did` (its DID document and grants).
    pub fn identity_graph(did: &str) -> String {
        format!("{did}/identity")
    }

    /// Writes the identity graph of the local actor `did` (DID document with public keys only,
    /// services, grants held) into the node's storage, replacing the previous state; stores
    /// with history keep the earlier state queryable.
    ///
    /// # Errors
    ///
    /// [`ProjectionError`] when an id is not an IRI or the store fails.
    // @tg: implements:: [[openspec:did-identity#DID Documents In The Runtime Graph]]
    pub fn publish_identity(&self, did: &str) -> Result<(), ProjectionError> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        let Some(entry) = self.identities.entry(did) else {
            return Ok(());
        };
        let doc = entry
            .actor
            .identity
            .document(entry.actor.km.as_ref())
            .map_err(|e| ProjectionError(e.to_string()))?;
        let mut g = project_actor(&doc, None)?;
        for t in project_grants(entry.policy.grants.grants(), Some(did))?.iter() {
            g.insert(t);
        }
        let graph = NamedNode::new(Self::identity_graph(did))
            .map_err(|e| ProjectionError(e.to_string()))?;
        let quads: Vec<Quad> = g
            .iter()
            .map(|t| t.into_owned().in_graph(GraphName::NamedNode(graph.clone())))
            .collect();
        let actor = NamedNode::new(did).map_err(|e| ProjectionError(e.to_string()))?;
        let store = storage.graph(&actor);
        // Replace only what changed, so the store's history shows a change, not a rewrite.
        let old = store
            .query(&format!(
                "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
                graph.as_str()
            ))
            .ok()
            .and_then(|r| r.into_solutions().ok());
        let mut existing: Vec<Quad> = Vec::new();
        if let Some(sols) = old {
            for i in 0..sols.len() {
                if let (Some(s), Some(oxrdf::Term::NamedNode(p)), Some(o)) =
                    (sols.get(i, "s"), sols.get(i, "p"), sols.get(i, "o"))
                {
                    let s: oxrdf::NamedOrBlankNode = match s {
                        oxrdf::Term::NamedNode(n) => n.clone().into(),
                        oxrdf::Term::BlankNode(b) => b.clone().into(),
                        _ => continue,
                    };
                    existing.push(Quad::new(s, p.clone(), o.clone(), graph.clone()));
                }
            }
        }
        let remove: Vec<Quad> = existing
            .iter()
            .filter(|q| !quads.contains(q))
            .cloned()
            .collect();
        let add: Vec<Quad> = quads
            .iter()
            .filter(|q| !existing.contains(q))
            .cloned()
            .collect();
        if !remove.is_empty() {
            store
                .remove_quads(&remove)
                .map_err(|e| ProjectionError(e.to_string()))?;
        }
        if !add.is_empty() {
            store
                .insert_quads(&add)
                .map_err(|e| ProjectionError(e.to_string()))?;
        }
        Ok(())
    }

    /// Picks up from every relay binding once.
    pub fn pickup(&self) -> crate::inbox::PickupReport {
        self.inbox.pickup()
    }

    /// Opens live pickup sessions where the relays offer them.
    pub fn go_live(&self) -> Vec<String> {
        self.inbox.go_live()
    }

    /// Works through due relay sends; returns how many finished.
    pub fn pump_outbox(&self) -> usize {
        self.outbox.pump()
    }

    /// One round of background work: pick up, then send.
    pub fn pump(&self) -> usize {
        let r = self.pickup();
        r.delivered.values().sum::<usize>() + r.refused + self.pump_outbox()
    }

    /// The relay outbox.
    pub fn outbox(&self) -> &RelayOutbox {
        &self.outbox
    }

    /// The relay inbox.
    pub fn inbox(&self) -> &RelayInbox {
        &self.inbox
    }

    /// The DID directory (resolution with caching).
    pub fn directory(&self) -> &DidDirectory {
        &self.directory
    }

    /// The caching resolver.
    pub fn resolver(&self) -> &Arc<CachingResolver> {
        &self.resolver
    }

    /// The local DID actors.
    pub fn identities(&self) -> &Arc<Identities> {
        &self.identities
    }

    /// Every inbound record.
    pub fn inbound_records(&self) -> Vec<InboundRecord> {
        self.inbox.records()
    }

    /// Every outbound record.
    pub fn outbound_records(&self) -> Vec<OutboundRecord> {
        self.outbox.records()
    }

    /// The clock.
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }
}

/// Inbound records grouped by actor.
pub fn records_by_actor(records: &[InboundRecord]) -> HashMap<String, Vec<InboundRecord>> {
    let mut m: HashMap<String, Vec<InboundRecord>> = HashMap::new();
    for r in records {
        m.entry(r.actor.clone()).or_default().push(r.clone());
    }
    m
}
