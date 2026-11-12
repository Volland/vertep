//! Shared fixtures: a mediator on an in-process network, VIDs that register with it, senders
//! that route through it.
#![allow(dead_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use url::Url;
use vertep_core::datamodel::{Event, EventType};
use vertep_federation::live::live_uri;
use vertep_federation::tsp::{TspIdentity, TspReceived, TspSession, Vid, vid_from_document};
use vertep_federation::tsp_mailbox::TspMailboxClient;
use vertep_federation::{
    ActorIdentity, Did, DidMethodChoice, DidResolver, IdentityConfig, InMemoryKeyManager,
    KeyManager, LocalLiveNetwork, LocalNetwork, ManualClock, NetError, RelayEndpoint,
    RelayEnvelope, RelayNetwork, ResolverRegistry, Service, ServiceEndpoint, Timestamp,
};
use vertep_store::oxrdf::NamedNode;
use vertep_store::{
    AppendError, AppendMeta, Entry, JournalCapabilities, JournalRecord, JournalStore, MemoryStore,
    Position, StoreError,
};
use vertep_tsp_mediator::{MailboxStore, Mediator, MediatorOptions};

/// The first mediator's endpoint.
pub const M1: &str = "https://m1.example/tsp";
/// The second mediator's endpoint.
pub const M2: &str = "https://m2.example/tsp";
/// The start of the clock: 2026-11-12T00:00:00Z.
pub const T0: i64 = 1_794_441_600_000;

/// A TSP service at `uri` routed through `via`.
pub fn tsp_service(uri: &str, via: &[String]) -> Service {
    Service {
        id: "#tsp".into(),
        types: vec!["TSPTransport".into()],
        endpoints: vec![ServiceEndpoint {
            uri: uri.into(),
            routing_keys: via.to_vec(),
            accept: Vec::new(),
        }],
    }
}

/// A `did:peer` identity publishing `services`.
pub fn peer(km: &dyn KeyManager, services: Vec<Service>) -> ActorIdentity {
    ActorIdentity::provision(km, &IdentityConfig::new(DidMethodChoice::Peer { services }))
        .expect("identity")
}

/// A journal store that can be made to fail its appends.
pub struct Flaky {
    pub inner: MemoryStore,
    pub fail: AtomicBool,
}

impl Flaky {
    pub fn new() -> Arc<Self> {
        Arc::new(Flaky {
            inner: MemoryStore::new(),
            fail: AtomicBool::new(false),
        })
    }
}

impl JournalStore for Flaky {
    fn capabilities(&self) -> JournalCapabilities {
        self.inner.capabilities()
    }
    fn append(
        &self,
        actor: &NamedNode,
        expected: Position,
        entries: &[Entry],
        meta: &AppendMeta,
    ) -> Result<Position, AppendError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(AppendError::Storage(StoreError::Backend(
                "disk full".into(),
            )));
        }
        self.inner.append(actor, expected, entries, meta)
    }
    fn read(&self, actor: &NamedNode, from: Position) -> Result<Vec<JournalRecord>, StoreError> {
        self.inner.read(actor, from)
    }
    fn head(&self, actor: &NamedNode) -> Result<Position, StoreError> {
        self.inner.head(actor)
    }
    fn actors(&self) -> Result<Vec<NamedNode>, StoreError> {
        self.inner.actors()
    }
}

/// An endpoint that records what reaches it (a recipient's own TSP endpoint).
#[derive(Default)]
pub struct Inbox(pub Mutex<Vec<Vec<u8>>>);

impl RelayEndpoint for Inbox {
    fn receive(&self, body: &[u8]) -> Result<Vec<u8>, NetError> {
        self.0.lock().unwrap().push(body.to_vec());
        Ok(Vec::new())
    }
}

/// The shared world: networks, resolver, clock.
pub struct World {
    pub net: Arc<LocalNetwork>,
    pub live: Arc<LocalLiveNetwork>,
    pub resolver: Arc<ResolverRegistry>,
    pub clock: Arc<ManualClock>,
}

impl World {
    pub fn new() -> Self {
        World {
            net: LocalNetwork::new(),
            live: LocalLiveNetwork::new(),
            resolver: Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1))),
            clock: Arc::new(ManualClock::new(Timestamp(T0))),
        }
    }

    /// A mediator at `uri` over `journal`, served on both networks.
    pub fn mediator_on(
        &self,
        uri: &str,
        journal: Arc<dyn JournalStore>,
        options: MediatorOptions,
    ) -> MediatorHandle {
        let km: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());
        let identity = peer(km.as_ref(), vec![tsp_service(uri, &[])]);
        self.mediator_with(uri, identity, km, journal, options)
    }

    /// A mediator with an existing identity (a restart, or another node of the same mediator).
    pub fn mediator_with(
        &self,
        uri: &str,
        identity: ActorIdentity,
        km: Arc<dyn KeyManager>,
        journal: Arc<dyn JournalStore>,
        options: MediatorOptions,
    ) -> MediatorHandle {
        let store = MailboxStore::open(journal.clone()).expect("store");
        let m = Mediator::new(
            identity.clone(),
            km.clone(),
            self.resolver.clone(),
            self.net.clone(),
            store,
            self.clock.clone(),
            options,
        )
        .expect("mediator");
        self.net.serve(uri, m.clone());
        self.live.serve(&live_uri(uri), m.clone());
        MediatorHandle {
            m,
            identity,
            km,
            journal,
        }
    }

    pub fn mediator(&self, uri: &str) -> MediatorHandle {
        self.mediator_on(
            uri,
            Arc::new(MemoryStore::new()),
            MediatorOptions::default(),
        )
    }

    /// A party with its own endpoint `uri`, routed through `via`.
    pub fn party(&self, uri: &str, via: &[String]) -> Party {
        let km: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());
        let identity = peer(km.as_ref(), vec![tsp_service(uri, via)]);
        let me = TspIdentity::new(&identity, km.clone(), Url::parse(uri).unwrap()).unwrap();
        Party {
            session: Arc::new(TspSession::new(me, true)),
            identity,
            uri: uri.to_owned(),
        }
    }

    pub fn vid(&self, did: &str) -> Vid {
        let doc = self
            .resolver
            .resolve(&Did::parse(did).unwrap())
            .unwrap()
            .document;
        vid_from_document(&doc).unwrap()
    }

    /// A pickup client of `mediator`, with live sessions.
    pub fn client(&self, mediator: &str) -> Arc<TspMailboxClient> {
        TspMailboxClient::with_live(
            mediator,
            self.net.clone(),
            self.resolver.clone(),
            self.live.clone(),
        )
        .expect("client")
    }

    /// Sends an event from `from` to `to` routed through `hops` (DIDs), posting to the first.
    pub fn send_routed(
        &self,
        from: &Party,
        to: &Party,
        hops: &[&str],
        first_uri: &str,
        id: &str,
    ) -> Result<Vec<u8>, NetError> {
        let hops: Vec<Vid> = hops.iter().map(|h| self.vid(h)).collect();
        let sealed = from
            .session
            .seal_routed(&self.vid(to.did()), &hops, &envelope(id))
            .expect("sealed");
        self.net.post(first_uri, &sealed.bytes)
    }
}

/// A mediator and what it was built from.
pub struct MediatorHandle {
    pub m: Arc<Mediator>,
    pub identity: ActorIdentity,
    pub km: Arc<dyn KeyManager>,
    pub journal: Arc<dyn JournalStore>,
}

impl MediatorHandle {
    pub fn did(&self) -> String {
        self.m.did().to_owned()
    }
}

/// A TSP party.
pub struct Party {
    pub session: Arc<TspSession>,
    pub identity: ActorIdentity,
    pub uri: String,
}

impl Party {
    pub fn did(&self) -> &str {
        &self.identity.did
    }

    /// Opens a delivered message and returns the event envelope with its verified sender.
    pub fn open(&self, w: &World, bytes: &[u8]) -> (String, RelayEnvelope) {
        let mut b = bytes.to_vec();
        let r = self
            .session
            .open(&mut b, &|id: &str| {
                let doc = w
                    .resolver
                    .resolve(&Did::parse(id).unwrap())
                    .unwrap()
                    .document;
                vid_from_document(&doc)
            })
            .expect("opens");
        match r {
            TspReceived::Event(m) => (m.origin.sender.clone().unwrap_or_default(), m.envelope),
            other => panic!("not an event: {other:?}"),
        }
    }
}

/// An event envelope with message id `id`.
pub fn envelope(id: &str) -> RelayEnvelope {
    let mut e = Event::named("order.placed", EventType::External);
    e.sendid = Some(format!("s-{id}"));
    RelayEnvelope::from_event(id, &e)
}
