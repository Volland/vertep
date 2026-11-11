//! Two organisations' runtime nodes joined by an in-process network, a test DIDComm mediator
//! and a test TSP intermediary: the fixture of the relay integration tests.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use vertep_core::datamodel::{DatamodelRegistry, Event, EventType, Value};
use vertep_core::ir::Chart;
use vertep_federation::directory::TransportKind;
use vertep_federation::federation::OUTBOX_ADDRESS;
use vertep_federation::{
    ActorIdentity, Did, DidMethodChoice, DidResolver, DropBox, Federation, IdentityConfig,
    InMemoryKeyManager, KeyManager, LocalActor, LocalNetwork, NetError, PickupBinding, Recipients,
    RelayNetwork, RelayProtocol, Resolution, ResolutionError, ResolverRegistry, RetryPolicy,
    RuntimeClock, Service, ServiceEndpoint,
};
use vertep_runtime::journal::EntryBody;
use vertep_runtime::{Address, Node, NodeConfig, Shell, Storage, VirtualClock};
use vertep_store::MemoryStore;

/// The start of the virtual clock: 2026-11-11T00:00:00Z.
pub const T0: i64 = 1_794_355_200_000;

/// The mediator's endpoint.
pub const MEDIATOR: &str = "https://mediator.example/didcomm";
/// The intermediary's endpoint.
pub const INTERMEDIARY: &str = "https://intermediary.example/tsp";

/// A chart type IRI.
pub fn ty(name: &str) -> String {
    format!("https://example.org/charts/{name}")
}

/// Reads an ECMAScript SCXML chart with the given body.
pub fn chart(initial: &str, body: &str) -> Chart {
    let xml = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="{initial}">{body}</scxml>"#
    );
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(&xml, &opts)
        .unwrap_or_else(|e| panic!("chart reads: {e:?}"))
        .chart
}

/// A sender: `go` sends `order.placed` (send id `s-17`, payload `{id, amount}`) to the DID in
/// `_event.data.to`; `send` sends `_event.data.event` to `_event.data.to` with id
/// `_event.data.id`; `local` sends `ping` to the `vt:` actor in `_event.data.to`. Everything it
/// receives is journaled.
pub fn sender() -> Chart {
    chart(
        "idle",
        r##"<datamodel><data id="lastid" expr="''"/><data id="errors" expr="[]"/></datamodel>
            <state id="idle">
              <transition event="go">
                <send event="order.placed" targetexpr="_event.data.to" id="s-17">
                  <content expr="({id: 'o-1', amount: 42})"/>
                </send>
              </transition>
              <transition event="send">
                <send eventexpr="_event.data.event" targetexpr="_event.data.to" idlocation="lastid">
                  <content expr="_event.data.payload"/>
                </send>
              </transition>
              <transition event="typed">
                <send eventexpr="_event.data.event" targetexpr="_event.data.to" typeexpr="_event.data.type"/>
              </transition>
              <transition event="mixed">
                <send event="ping" targetexpr="_event.data.local"/>
                <send event="order.placed" targetexpr="_event.data.remote" id="s-remote"/>
              </transition>
              <transition event="*"/>
            </state>"##,
    )
}

/// A receiver that journals everything; `approve` from the DID in `approver` moves it to
/// `approved`.
pub fn receiver() -> Chart {
    chart(
        "idle",
        r##"<datamodel><data id="approver" expr="''"/></datamodel>
            <state id="idle">
              <transition event="set.approver"><assign location="approver" expr="_event.data"/></transition>
              <transition event="approve" cond="_event.origin === approver" target="approved"/>
              <transition event="*"/>
            </state>
            <state id="approved"><transition event="*"/></state>"##,
    )
}

/// The ECMAScript registry.
pub fn ecma() -> Arc<dyn DatamodelRegistry + Send + Sync> {
    Arc::new(vertep_dm_ecma::EcmaRegistry)
}

/// An external event with data.
pub fn ev(name: &str, data: Value) -> Event {
    let mut e = Event::named(name, EventType::External);
    e.data = data;
    e
}

/// A JSON object value.
pub fn obj(kv: &[(&str, Value)]) -> Value {
    Value::Object(
        kv.iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

/// A string value.
pub fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

/// The input events journaled by `a`, in order.
pub fn inputs(node: &Node, a: &Address) -> Vec<Event> {
    node.journal(a)
        .map(|j| {
            j.iter()
                .filter_map(|e| match &e.body {
                    EntryBody::Input { input, .. } => Some(input.event.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The names of the input events journaled by `a`.
pub fn names(node: &Node, a: &Address) -> Vec<String> {
    inputs(node, a).into_iter().map(|e| e.name).collect()
}

/// A DID address.
pub fn addr(did: &str) -> Address {
    Address::parse(did).expect("DID is an IRI")
}

/// A DIDComm service through `routing_keys` at `uri`.
pub fn didcomm_service(uri: &str, routing_keys: &[String]) -> Service {
    Service {
        id: "#didcomm".into(),
        types: vec!["DIDCommMessaging".into()],
        endpoints: vec![ServiceEndpoint {
            uri: uri.into(),
            routing_keys: routing_keys.to_vec(),
            accept: vec!["didcomm/v2".into()],
        }],
    }
}

/// A TSP service at `uri`, routed through `intermediaries` when not empty.
pub fn tsp_service(uri: &str, intermediaries: &[String]) -> Service {
    Service {
        id: "#tsp".into(),
        types: vec!["TSPTransport".into()],
        endpoints: vec![ServiceEndpoint {
            uri: uri.into(),
            routing_keys: intermediaries.to_vec(),
            accept: Vec::new(),
        }],
    }
}

/// A resolver that counts resolutions and can be made to fail.
pub struct Counting {
    pub inner: Arc<dyn DidResolver>,
    pub calls: AtomicUsize,
    pub fail: Mutex<Option<ResolutionError>>,
}

impl Counting {
    pub fn new(inner: Arc<dyn DidResolver>) -> Arc<Self> {
        Arc::new(Counting {
            inner,
            calls: AtomicUsize::new(0),
            fail: Mutex::new(None),
        })
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DidResolver for Counting {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = self.fail.lock().expect("lock").clone() {
            return Err(e);
        }
        self.inner.resolve(did)
    }
}

/// A network that delivers the first `n` posts to a URI and then reports them as timed out
/// (the response was lost), as a sender that retries after a timeout sees it.
pub struct LostResponses {
    pub inner: Arc<LocalNetwork>,
    pub uri: String,
    pub remaining: AtomicUsize,
}

impl RelayNetwork for LostResponses {
    fn post(&self, endpoint: &str, body: &[u8]) -> Result<Vec<u8>, NetError> {
        let r = self.inner.post(endpoint, body)?;
        if endpoint == self.uri
            && self
                .remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            return Err(NetError::Unreachable(format!("{endpoint}: timed out")));
        }
        Ok(r)
    }
}

/// One organisation: a runtime node and its federation layer.
pub struct Org {
    pub node: Node,
    pub fed: Federation,
    pub store: Arc<dyn Storage>,
    pub km: Arc<InMemoryKeyManager>,
}

/// How to build an organisation's node.
pub struct OrgSpec {
    pub bindings: Vec<PickupBinding>,
    pub protocols: Vec<Arc<dyn RelayProtocol>>,
    pub preference: Vec<TransportKind>,
    pub net: Option<Arc<dyn RelayNetwork>>,
    pub resolver: Option<Arc<dyn DidResolver>>,
    pub config: NodeConfig,
    pub store: Option<Arc<dyn Storage>>,
    pub km: Option<Arc<InMemoryKeyManager>>,
}

impl Default for OrgSpec {
    fn default() -> Self {
        OrgSpec {
            bindings: Vec::new(),
            protocols: Vec::new(),
            preference: vec![TransportKind::Tsp, TransportKind::DidComm],
            net: None,
            resolver: None,
            config: NodeConfig::default(),
            store: None,
            km: None,
        }
    }
}

/// Hosted DID documents (`did:web`), served from memory as a publisher would.
#[derive(Default)]
pub struct Docs(pub Mutex<std::collections::HashMap<String, vertep_federation::DidDocument>>);

impl Docs {
    pub fn put(&self, doc: vertep_federation::DidDocument) {
        self.0.lock().expect("lock").insert(doc.id.clone(), doc);
    }
}

impl DidResolver for Docs {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        self.0
            .lock()
            .expect("lock")
            .get(did.as_str())
            .cloned()
            .map(Resolution::of)
            .ok_or_else(|| ResolutionError::NotFound(did.to_string()))
    }
}

/// The shared world: network, clock, resolvers.
pub struct World {
    pub net: Arc<LocalNetwork>,
    pub clock: VirtualClock,
    pub registry: Arc<ResolverRegistry>,
    pub docs: Arc<Docs>,
}

impl World {
    pub fn new() -> Self {
        let registry = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
        let docs = Arc::new(Docs::default());
        registry.register("web", docs.clone());
        World {
            net: LocalNetwork::new(),
            clock: VirtualClock::new(T0),
            registry,
            docs,
        }
    }

    pub fn shell(&self) -> Shell {
        Shell {
            spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
            clock: Arc::new(self.clock.clone()),
            sleep: Arc::new(self.clock.clone()),
        }
    }

    /// A node with a federation layer per `spec`, both chart types registered.
    pub fn org(&self, spec: OrgSpec) -> Org {
        let store: Arc<dyn Storage> = spec
            .store
            .unwrap_or_else(|| Arc::new(MemoryStore::new()) as Arc<dyn Storage>);
        let km = spec
            .km
            .unwrap_or_else(|| Arc::new(InMemoryKeyManager::new()));
        let net = spec
            .net
            .unwrap_or_else(|| self.net.clone() as Arc<dyn RelayNetwork>);
        let resolver = spec
            .resolver
            .unwrap_or_else(|| self.registry.clone() as Arc<dyn DidResolver>);
        let mut b = Federation::builder(resolver, net)
            .clock(Arc::new(RuntimeClock(Arc::new(self.clock.clone()))))
            .preference(spec.preference)
            .storage(store.clone())
            .retry(RetryPolicy {
                max_attempts: 3,
                backoff_ms: 1_000,
                max_backoff_ms: 10_000,
            });
        for p in spec.protocols {
            b = b.protocol(p);
        }
        for binding in spec.bindings {
            b = b.pickup(binding);
        }
        let fed = b.build();
        let mut nb = Node::builder(store.clone(), self.shell())
            .datamodels(ecma())
            .config(spec.config)
            .mailboxes_for_scheme("did", fed.mailboxes());
        for k in [TransportKind::DidComm, TransportKind::Tsp] {
            nb = nb.transport(fed.transport(k));
        }
        let mut node = nb.build();
        node.register_type(&ty("sender"), sender()).expect("sender");
        node.register_type(&ty("receiver"), receiver())
            .expect("receiver");
        fed.attach(&node.handle());
        Org {
            node,
            fed,
            store,
            km,
        }
    }
}

impl Org {
    /// Provisions a `did:peer` actor of `chart` with `key`, publishing `services`.
    pub fn did_actor(&self, chart: &str, key: &str, services: Vec<Service>) -> ActorIdentity {
        self.fed
            .provision(
                &self.node.handle(),
                &ty(chart),
                key,
                &IdentityConfig::new(DidMethodChoice::Peer { services }),
                self.km.clone() as Arc<dyn KeyManager>,
            )
            .expect("provisioned")
    }

    /// Hosts an existing identity again (after a restart).
    pub fn rehost(&self, chart: &str, key: &str, identity: &ActorIdentity) {
        self.node
            .handle()
            .bind_address(&addr(&identity.did), &ty(chart), key);
        self.fed.host(
            LocalActor {
                identity: identity.clone(),
                km: self.km.clone() as Arc<dyn KeyManager>,
            },
            Default::default(),
            Arc::new(vertep_federation::origin::Untyped),
        );
    }
}

/// Runs both organisations until nothing moves.
pub fn settle(orgs: &mut [&mut Org]) {
    for _ in 0..50 {
        let mut progress = 0;
        for o in orgs.iter_mut() {
            progress += o.node.run_until_idle();
            progress += o.fed.pump();
        }
        if progress == 0 {
            return;
        }
    }
}

/// A mediator binding for `recipients`.
#[cfg(feature = "didcomm")]
pub fn mediator_binding(
    m: &Arc<vertep_federation::testkit::TestMediator>,
    recipients: Recipients,
) -> PickupBinding {
    PickupBinding {
        name: "mediator".into(),
        source: m.clone(),
        protocol: Arc::new(vertep_federation::didcomm::DidCommProtocol),
        recipients,
    }
}

/// A test mediator served on the world's network.
#[cfg(feature = "didcomm")]
pub fn mediator(w: &World) -> Arc<vertep_federation::testkit::TestMediator> {
    let m = vertep_federation::testkit::TestMediator::new(
        Arc::new(InMemoryKeyManager::new()),
        MEDIATOR,
        w.net.clone(),
        w.registry.clone(),
    )
    .expect("mediator");
    w.net.serve(MEDIATOR, m.clone());
    m
}

/// A pickup binding over an inbound drop box.
pub fn drop_binding(
    name: &str,
    b: &Arc<DropBox>,
    protocol: Arc<dyn RelayProtocol>,
) -> PickupBinding {
    PickupBinding {
        name: name.into(),
        source: b.clone(),
        protocol,
        recipients: Recipients::Shared,
    }
}

/// The address of the relay outbox queue.
pub fn outbox_address() -> Address {
    Address::parse(OUTBOX_ADDRESS).expect("IRI")
}
