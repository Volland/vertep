//! Protocol spaces over real relays: the join policy, pairwise sealed fan-out through a DIDComm
//! mediator, TSP endpoints and an in-process channel, and the space requirements end to end
//! between agents with `did:peer` identities and key managers.
#![cfg(all(feature = "tsp", feature = "didcomm", feature = "testkit"))]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use vertep_federation::didcomm::DidCommProtocol;
use vertep_federation::directory::TransportKind;
use vertep_federation::origin::Untyped;
use vertep_federation::protocol::{
    ActorSigner, JoinPolicy, JoinRefusal, JoinRule, LocalChannel, Outcome, Refusal, SpaceMember,
    SpaceTransport, Via, message_of,
};
use vertep_federation::testkit::TestMediator;
use vertep_federation::tsp::TspProtocol;
use vertep_federation::{
    ActorIdentity, Did, DidMethodChoice, DidResolver, DropBox, Federation, Grant, IdentityConfig,
    InMemoryKeyManager, KeyManager, LocalActor, LocalNetwork, RelayNetwork, RelayProtocol,
    Resolution, ResolutionError, ResolverRegistry, Service, ServiceEndpoint, Timestamp,
};
use vertep_kolyada::{Body, Message, Protocol, Reason, Status, control};

const MEDIATOR: &str = "https://mediator.example/didcomm";
const NOW: Timestamp = Timestamp(1_794_355_200_000);

fn quote() -> Arc<Protocol> {
    let doc = json!({
        "@context": "https://w3id.org/vertep/kolyada/1.0",
        "type": "Protocol",
        "version": "1.0",
        "roles": ["buyer", "seller", "auditor"],
        "messages": [
            { "id": "quote.request", "payload": { "fields": [
                { "name": "item", "datatype": "string", "min": 1, "max": 1 },
                { "name": "qty", "datatype": "integer", "min": 1, "max": 1 }
            ] } },
            { "id": "quote", "payload": { "fields": [
                { "name": "price", "datatype": "number", "min": 1, "max": 1 },
                { "name": "currency", "datatype": "string", "in": ["EUR", "USD"] }
            ] } },
            { "id": "order" },
            { "id": "cancel" },
            { "id": "receipt" }
        ],
        "initial": "Idle",
        "final": ["Cancelled", "Done"],
        "transitions": [
            { "from": "Idle", "sender": "buyer", "receivers": ["seller"], "message": "quote.request", "to": "Quoting" },
            { "from": "Quoting", "sender": "seller", "receivers": ["buyer", "auditor"], "message": "quote", "to": "Decided" },
            { "from": "Decided", "sender": "buyer", "receivers": ["seller"], "message": "order", "to": "Ordered" },
            { "from": "Decided", "sender": "buyer", "receivers": ["seller"], "message": "cancel", "to": "Cancelled" },
            { "from": "Ordered", "sender": "seller", "receivers": ["buyer"], "message": "receipt", "to": "Done" }
        ],
        "limits": { "bufferSize": 4, "timeoutMs": 1000 }
    });
    Arc::new(Protocol::parse(doc.to_string().as_bytes()).expect("quote parses"))
}

/// A payment sub-protocol: the payer pays, the payee confirms; the bank, when bound, sees the
/// payment.
fn payment(with_bank: bool) -> Arc<Protocol> {
    let (roles, pay_to) = if with_bank {
        (json!(["payer", "payee", "bank"]), json!(["payee", "bank"]))
    } else {
        (json!(["payer", "payee"]), json!(["payee"]))
    };
    let doc = json!({
        "@context": "https://w3id.org/vertep/kolyada/1.0",
        "type": "Protocol", "version": "1.0",
        "roles": roles,
        "messages": [{ "id": "pay" }, { "id": "paid" }],
        "initial": "Open", "final": ["Settled"],
        "transitions": [
            { "from": "Open", "sender": "payer", "receivers": pay_to, "message": "pay", "to": "Paid" },
            { "from": "Paid", "sender": "payee", "receivers": ["payer"], "message": "paid", "to": "Settled" }
        ]
    });
    Arc::new(Protocol::parse(doc.to_string().as_bytes()).expect("payment parses"))
}

/// How an agent can be reached.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    /// A TSP endpoint and a DIDComm service through the mediator.
    Both,
    /// The mediator only.
    DidComm,
    /// The in-process channel only.
    Local,
}

/// A resolver that counts resolutions: the cost of a check.
struct Counting {
    inner: Arc<dyn DidResolver>,
    calls: AtomicUsize,
}

impl DidResolver for Counting {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve(did)
    }
}

struct Agent {
    actor: LocalActor,
    member: SpaceMember,
    inbound: Arc<DropBox>,
    verifications: Arc<Counting>,
}

impl Agent {
    fn did(&self) -> String {
        self.actor.identity.did.clone()
    }
}

/// Agents on one in-process network with a DIDComm mediator, TSP endpoints and a local channel.
struct World {
    net: Arc<LocalNetwork>,
    registry: Arc<ResolverRegistry>,
    mediator: Arc<TestMediator>,
    tsp: Arc<TspProtocol>,
    fed: Federation,
    local: Arc<LocalChannel>,
    agents: BTreeMap<&'static str, Agent>,
}

impl World {
    fn new() -> Self {
        let net = LocalNetwork::new();
        let registry = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
        let mediator = TestMediator::new(
            Arc::new(InMemoryKeyManager::new()),
            MEDIATOR,
            net.clone(),
            registry.clone(),
        )
        .expect("mediator");
        net.serve(MEDIATOR, mediator.clone());
        let fed = Federation::builder(
            registry.clone() as Arc<dyn DidResolver>,
            net.clone() as Arc<dyn RelayNetwork>,
        )
        .build();
        World {
            net,
            registry,
            mediator,
            tsp: Arc::new(TspProtocol::new(true)),
            fed,
            local: LocalChannel::new(),
            agents: BTreeMap::new(),
        }
    }

    fn add(&mut self, name: &'static str, reach: Reach) -> String {
        let km = Arc::new(InMemoryKeyManager::new());
        let tsp_uri = format!("https://{name}.example/tsp");
        let didcomm = Service {
            id: "#didcomm".into(),
            types: vec!["DIDCommMessaging".into()],
            endpoints: vec![ServiceEndpoint {
                uri: MEDIATOR.into(),
                routing_keys: vec![self.mediator.routing_key()],
                accept: vec!["didcomm/v2".into()],
            }],
        };
        let tsp = Service {
            id: "#tsp".into(),
            types: vec!["TSPTransport".into()],
            endpoints: vec![ServiceEndpoint::uri(tsp_uri.clone())],
        };
        let services = match reach {
            Reach::Both => vec![tsp, didcomm],
            Reach::DidComm => vec![didcomm],
            Reach::Local => Vec::new(),
        };
        let identity = ActorIdentity::provision(
            km.as_ref(),
            &IdentityConfig::new(DidMethodChoice::Peer { services }),
        )
        .expect("identity");
        let actor = LocalActor {
            identity,
            km: km as Arc<dyn KeyManager>,
        };
        self.fed
            .host(actor.clone(), Default::default(), Arc::new(Untyped));
        let inbound = DropBox::new(name);
        if reach == Reach::Both {
            self.net.serve(
                &tsp_uri,
                Arc::new(self.tsp.endpoint(
                    self.fed.identities().clone(),
                    inbound.clone(),
                    self.registry.clone(),
                )),
            );
        }
        if reach == Reach::Local {
            self.local.host(&actor.identity.did);
        }
        let verifications = Arc::new(Counting {
            inner: self.registry.clone(),
            calls: AtomicUsize::new(0),
        });
        let member = SpaceMember::new(
            Arc::new(ActorSigner(actor.clone())),
            Arc::new(vertep_federation::protocol::ResolverVerifier(
                verifications.clone() as Arc<dyn DidResolver>,
            )),
        );
        let did = actor.identity.did.clone();
        self.agents.insert(
            name,
            Agent {
                actor,
                member,
                inbound,
                verifications,
            },
        );
        did
    }

    fn did(&self, name: &str) -> String {
        self.agents[name].did()
    }

    fn a(&mut self, name: &str) -> &mut Agent {
        self.agents.get_mut(name).expect("agent")
    }

    /// A transport over the relays, tried in `preference` order, with the local channel.
    fn transport(&self, preference: &[TransportKind]) -> SpaceTransport {
        SpaceTransport::new(
            self.registry.clone() as Arc<dyn DidResolver>,
            self.net.clone() as Arc<dyn RelayNetwork>,
        )
        .protocol(self.tsp.clone() as Arc<dyn RelayProtocol>)
        .protocol(Arc::new(DidCommProtocol))
        .preference(preference.to_vec())
        .local(self.local.clone())
    }

    /// Fans `msg` out from `from` over the relays, TSP first.
    fn send(&self, from: &str, msg: &Message) -> vertep_federation::protocol::FanOut {
        self.send_with(from, msg, &[TransportKind::Tsp, TransportKind::DidComm])
    }

    fn send_with(
        &self,
        from: &str,
        msg: &Message,
        preference: &[TransportKind],
    ) -> vertep_federation::protocol::FanOut {
        let a = &self.agents[from];
        let report = self
            .transport(preference)
            .fan_out(&a.actor, msg, &a.member.bindings_for(msg));
        assert!(report.complete(), "fan-out failed: {:?}", report.failed);
        report
    }

    /// One agent's pickups: the mediator, its TSP inbound queue and the local channel.
    fn pickup(&mut self, name: &str) -> Vec<Outcome> {
        let a = self.agents.get_mut(name).expect("agent");
        let mut out = a.member.pickup(
            &a.actor,
            self.mediator.as_ref(),
            &DidCommProtocol,
            self.registry.as_ref(),
            NOW,
        );
        out.extend(a.member.pickup(
            &a.actor,
            a.inbound.as_ref(),
            self.tsp.as_ref(),
            self.registry.as_ref(),
            NOW,
        ));
        out.extend(a.member.pickup_local(&self.local, 0));
        out
    }

    /// Runs every agent's pickups until nothing settles any more.
    fn settle(&mut self) -> BTreeMap<&'static str, Vec<Outcome>> {
        let names: Vec<&'static str> = self.agents.keys().copied().collect();
        let mut all: BTreeMap<&'static str, Vec<Outcome>> = BTreeMap::new();
        for _ in 0..20 {
            let mut progress = false;
            for n in &names {
                let out = self.pickup(n);
                progress |= out
                    .iter()
                    .any(|o| !matches!(o, Outcome::Held(_) | Outcome::Deferred(_)));
                all.entry(n).or_default().extend(out);
            }
            if !progress {
                break;
            }
        }
        all
    }
}

fn admit(protocol: &Protocol) -> JoinPolicy {
    JoinPolicy::default().allow(JoinRule::protocol(protocol.id().as_str()))
}

fn quote_bindings(w: &World) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("buyer".to_owned(), w.did("buyer")),
        ("seller".to_owned(), w.did("seller")),
        ("auditor".to_owned(), w.did("auditor")),
    ])
}

/// Buyer (TSP and DIDComm), seller (TSP and DIDComm) and auditor (DIDComm only), each admitting
/// the quote protocol; the buyer's genesis has reached both others. Returns the genesis.
fn genesis_delivered(w: &mut World) -> Message {
    let p = quote();
    for n in ["buyer", "seller", "auditor"] {
        let a = w.a(n);
        a.member.know(p.clone());
        a.member.set_policy(admit(&p));
    }
    let bindings = quote_bindings(w);
    let genesis = w
        .a("buyer")
        .member
        .open(p, "buyer", &bindings, None)
        .expect("genesis");
    w.send("buyer", &genesis);
    w.settle();
    genesis
}

fn three() -> World {
    let mut w = World::new();
    w.add("buyer", Reach::Both);
    w.add("seller", Reach::Both);
    w.add("auditor", Reach::DidComm);
    w
}

/// The same with every role joined: the instance is open everywhere. Returns its id.
fn open_space(w: &mut World) -> String {
    let id = genesis_delivered(w).hash();
    for n in ["seller", "auditor"] {
        let j = w.a(n).member.join(&id).expect("join");
        w.send(n, &j);
    }
    w.settle();
    id
}

fn journaled(o: &[Outcome], hash: &str) -> bool {
    o.iter()
        .any(|o| matches!(o, Outcome::Journaled { hash: h, .. } if h == hash))
}

/// A message signed by `from` outside what its projection allows: no send path would make it.
fn forged(w: &World, from: &str, role: &str, id: &str, kind: &str, to: &[&str]) -> Message {
    let a = &w.agents[from];
    let inst = a.member.instance(id).expect("instance");
    let body = Body {
        protocol: inst.protocol().id().as_str().into(),
        instance: Some(id.into()),
        sender: a.did(),
        role: role.into(),
        seq: 40,
        deps: inst.heads(),
        recipients: to.iter().map(|r| (*r).to_owned()).collect(),
        kind: kind.into(),
        payload: json!({}),
    };
    Message::sign(body, &ActorSigner(a.actor.clone())).expect("signs")
}

// ---------------------------------------------------------------------------------------------
// Join Policy
// ---------------------------------------------------------------------------------------------

/// A genesis from the buyer to a seller reached over `reach`.
fn genesis_to(reach: Reach) -> (World, Message) {
    let mut w = World::new();
    w.add("buyer", Reach::Both);
    w.add("seller", reach);
    let p = quote();
    w.a("seller").member.know(p.clone());
    let bindings = BTreeMap::from([
        ("buyer".to_owned(), w.did("buyer")),
        ("seller".to_owned(), w.did("seller")),
        ("auditor".to_owned(), w.did("buyer")),
    ]);
    let genesis = w
        .a("buyer")
        .member
        .open(p, "buyer", &bindings, None)
        .expect("genesis");
    (w, genesis)
}

// @tg: verifies:: [[openspec:protocol-spaces#Join Policy#Default refuses]] {level: integration}
#[test]
fn default_refuses() {
    for reach in [Reach::Both, Reach::DidComm, Reach::Local] {
        let (mut w, genesis) = genesis_to(reach);
        let report = w.send("buyer", &genesis);
        assert_eq!(report.sent.len(), 1);
        let out = w.pickup("seller");
        assert!(
            matches!(
                out.as_slice(),
                [Outcome::Refused {
                    refusal: Refusal::Policy(JoinRefusal::Default),
                    ..
                }]
            ),
            "{out:?}"
        );
        let s = &w.agents["seller"];
        assert!(s.member.journal().is_empty(), "nothing is appended");
        assert!(s.member.instance(&genesis.hash()).is_none());
        assert_eq!(
            s.verifications.calls.load(Ordering::SeqCst),
            0,
            "a refused genesis costs no resolution or signature check"
        );
        // The refusal is recorded and the copy acknowledged, so the relay holds nothing more.
        assert_eq!(s.member.refusals().len(), 1);
        let did = s.did();
        assert!(w.mediator.store().held_for(&did).is_empty());
        assert!(s.inbound.held().is_empty());
        assert!(w.local.held_for(&did).is_empty());
        // Later messages of the refused instance are refused too, still without a journal entry.
        assert!(w.pickup("seller").is_empty());
    }
}

// @tg: verifies:: [[openspec:protocol-spaces#Join Policy#Policy admits a known initiator]] {level: integration}
#[test]
fn policy_admits_a_known_initiator() {
    for reach in [Reach::Both, Reach::DidComm, Reach::Local] {
        let (mut w, genesis) = genesis_to(reach);
        let buyer = w.did("buyer");
        let policy = JoinPolicy::default().allow(
            JoinRule::protocol(quote().id().as_str())
                .role("seller")
                .initiator(buyer),
        );
        w.a("seller").member.set_policy(policy);
        w.send("buyer", &genesis);
        let out = w.pickup("seller");
        assert!(journaled(&out, &genesis.hash()), "{out:?}");
        let s = &w.agents["seller"];
        assert_eq!(s.member.journal().len(), 1);
        assert_eq!(s.member.journal()[0].message, genesis);
        let inst = s.member.instance(&genesis.hash()).expect("instance");
        assert_eq!(inst.local_role(), "seller");
        assert_eq!(inst.status(), Status::Pending);
    }
}

#[test]
fn policy_refuses_what_it_does_not_name() {
    let (w, genesis) = genesis_to(Reach::Both);
    let seller = w.did("seller");
    let p = quote();
    let id = p.id().as_str().to_owned();
    let refuse = |policy: JoinPolicy| policy.decide(&genesis, &seller).unwrap_err();
    assert_eq!(
        refuse(JoinPolicy::default().allow(JoinRule::protocol("urn:vertep:protocol:sha256:00"))),
        JoinRefusal::Protocol(id.clone())
    );
    assert!(matches!(
        refuse(JoinPolicy::default().allow(JoinRule::protocol(&id).role("auditor"))),
        JoinRefusal::Role(_)
    ));
    assert!(matches!(
        refuse(JoinPolicy::default().allow(JoinRule::protocol(&id).initiator("did:example:x"))),
        JoinRefusal::Initiator(_)
    ));
    let no_credential: Arc<dyn vertep_federation::protocol::CredentialCheck> =
        Arc::new(|_: &Message| false);
    assert!(matches!(
        refuse(JoinPolicy::default().allow(JoinRule::protocol(&id).credential(no_credential))),
        JoinRefusal::Credential(_)
    ));
    assert_eq!(
        JoinPolicy::default()
            .allow(JoinRule::protocol(&id))
            .decide(&genesis, "did:example:other"),
        Err(JoinRefusal::NotBound)
    );
    let with_credential: Arc<dyn vertep_federation::protocol::CredentialCheck> =
        Arc::new(|g: &Message| g.body.role == "buyer");
    assert_eq!(
        JoinPolicy::default()
            .allow(JoinRule::protocol(&id).credential(with_credential))
            .decide(&genesis, &seller),
        Ok(())
    );
}

// ---------------------------------------------------------------------------------------------
// Protocol Instance and Joining
// ---------------------------------------------------------------------------------------------

// @tg: verifies:: [[openspec:protocol-spaces#Protocol Instance#Genesis creates an instance]] {level: integration}
#[test]
fn genesis_creates_an_instance_over_relays() {
    let mut w = three();
    let genesis = genesis_delivered(&mut w);
    let bindings = quote_bindings(&w);
    for n in ["buyer", "seller", "auditor"] {
        let inst = w.agents[n]
            .member
            .instance(&genesis.hash())
            .expect("instance");
        assert_eq!(inst.id(), genesis.hash());
        assert_eq!(inst.bindings(), &bindings);
        assert_eq!(inst.local_role(), n);
    }
}

// @tg: verifies:: [[openspec:protocol-spaces#Protocol Instance#Unsigned genesis refused]] {level: integration}
#[test]
fn unsigned_genesis_refused_over_relays() {
    for reach in [Reach::Both, Reach::DidComm, Reach::Local] {
        let (mut w, genesis) = genesis_to(reach);
        w.a("seller").member.set_policy(admit(&quote()));
        let mut forged = genesis.clone();
        forged.signature = "AAAA".into();
        w.send("buyer", &forged);
        let out = w.pickup("seller");
        assert!(
            matches!(
                out.as_slice(),
                [Outcome::Refused {
                    refusal: Refusal::Check(Reason::Signature),
                    ..
                }]
            ),
            "{out:?}"
        );
        assert!(w.agents["seller"].member.journal().is_empty());
    }
}

// @tg: verifies:: [[openspec:protocol-spaces#Joining#Instance opens]] {level: integration}
#[test]
fn instance_opens_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    for n in ["buyer", "seller", "auditor"] {
        assert_eq!(
            w.agents[n].member.instance(&id).expect("instance").status(),
            Status::Open,
            "{n}"
        );
    }
    // Each journal holds the genesis and both joins.
    for n in ["buyer", "seller", "auditor"] {
        assert_eq!(w.agents[n].member.journal().len(), 3, "{n}");
    }
}

// @tg: verifies:: [[openspec:protocol-spaces#Joining#Decline ends the instance]] {level: integration}
#[test]
fn decline_ends_the_instance_over_relays() {
    let mut w = three();
    let id = genesis_delivered(&mut w).hash();
    let d = w.a("auditor").member.decline(&id).expect("decline");
    w.send("auditor", &d);
    let j = w.a("seller").member.join(&id).expect("join");
    w.send("seller", &j);
    w.settle();
    let buyer = w.agents["buyer"].member.instance(&id).expect("instance");
    assert_eq!(buyer.status(), Status::Declined, "the initiator is told");
    assert!(buyer.has_applied(&d.hash()));
}

// @tg: verifies:: [[openspec:protocol-spaces#Joining#No join, no standing]] {level: integration}
#[test]
fn no_join_no_standing_over_relays() {
    let mut w = three();
    let id = genesis_delivered(&mut w).hash();
    // The seller has not joined; a quote it signs anyway is refused by the buyer.
    let m = forged(&w, "seller", "seller", &id, "quote", &["buyer", "auditor"]);
    w.send("seller", &m);
    let out = w.settle();
    assert!(
        out["buyer"].iter().any(|o| matches!(
            o,
            Outcome::Refused {
                refusal: Refusal::Check(Reason::NotOpen),
                ..
            }
        )),
        "{:?}",
        out["buyer"]
    );
    assert_eq!(
        w.agents["buyer"].member.journal().len(),
        1,
        "the genesis only"
    );
}

// ---------------------------------------------------------------------------------------------
// Role-Derived Authority
// ---------------------------------------------------------------------------------------------

fn request(w: &mut World, id: &str) -> Message {
    let m = w
        .a("buyer")
        .member
        .send(
            id,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .expect("request");
    w.send("buyer", &m);
    m
}

// @tg: verifies:: [[openspec:protocol-spaces#Role-Derived Authority#Role grants exactly the projection]] {level: integration}
#[test]
fn role_grants_exactly_the_projection_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    // No grant is configured for any DID.
    let m = request(&mut w, &id);
    let out = w.settle();
    assert!(journaled(&out["seller"], &m.hash()), "{:?}", out["seller"]);
    // The same role, past the state that allows a request, sends one more: refused.
    let again = forged(&w, "buyer", "buyer", &id, "quote.request", &["seller"]);
    w.send("buyer", &again);
    let out = w.settle();
    assert!(out["seller"].iter().any(|o| matches!(
        o,
        Outcome::Refused {
            refusal: Refusal::Check(Reason::NotEnabledInState),
            ..
        }
    )));
}

// @tg: verifies:: [[openspec:protocol-spaces#Role-Derived Authority#Grant does not widen]] {level: integration}
#[test]
fn grant_does_not_widen_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    // A DID grant lets the buyer send every event to the seller.
    assert!(
        w.fed
            .grant_inbound(Grant::new(w.did("buyer"), w.did("seller"), ["*"]))
    );
    request(&mut w, &id);
    w.settle();
    // `order` is the buyer's only after a quote; the grant does not widen the role.
    let order = forged(&w, "buyer", "buyer", &id, "order", &["seller"]);
    w.send("buyer", &order);
    let out = w.settle();
    assert!(
        out["seller"].iter().any(|o| matches!(
            o,
            Outcome::Refused {
                refusal: Refusal::Check(Reason::NotEnabledInState),
                ..
            }
        )),
        "{:?}",
        out["seller"]
    );
    assert!(
        !w.agents["seller"]
            .member
            .journal()
            .iter()
            .any(|e| e.message.hash() == order.hash())
    );
}

// ---------------------------------------------------------------------------------------------
// Pairwise Sealed Fan-Out
// ---------------------------------------------------------------------------------------------

/// Every message `actor` can open among the copies the relays hold for anyone, and among every
/// post on the wire.
fn readable_by(w: &World, actor: &LocalActor) -> Vec<Message> {
    let mut copies: Vec<Vec<u8>> = w
        .mediator
        .store()
        .held()
        .into_iter()
        .map(|m| m.bytes)
        .collect();
    for a in w.agents.values() {
        copies.extend(a.inbound.held().into_iter().map(|m| m.bytes));
    }
    copies.extend(w.net.posts().into_iter().map(|(_, b)| b));
    // A separate TSP protocol, so opening relationship messages here leaves the agents'
    // relationships alone.
    let tsp = TspProtocol::new(false);
    let protocols: [&dyn RelayProtocol; 2] = [&DidCommProtocol, &tsp];
    let mut out = Vec::new();
    for bytes in copies {
        for p in protocols {
            if let Ok(opened) = p.open(actor, &bytes, w.registry.as_ref(), NOW)
                && let Ok(m) = message_of(&opened.envelope)
            {
                out.push(m);
            }
        }
    }
    out
}

// @tg: verifies:: [[openspec:protocol-spaces#Pairwise Sealed Fan-Out#Only the recipient can read]] {level: integration}
#[test]
fn only_the_recipient_can_read_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    // The request is sealed to the seller only: buyer and seller read it, the auditor never.
    let m = w
        .a("buyer")
        .member
        .send(
            &id,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .expect("request");
    let report = w.send("buyer", &m);
    assert_eq!(report.sent.len(), 1, "one copy, for the seller");
    assert_eq!(report.sent[0].did, w.did("seller"));
    assert_eq!(report.sent[0].via, Via::Relay(TransportKind::Tsp));
    // Every relay copy held for the auditor is sealed to it, and none of them is the request.
    let auditor = w.agents["auditor"].actor.clone();
    let seller = w.agents["seller"].actor.clone();
    for held in w.mediator.store().held_for(&auditor.identity.did) {
        assert!(
            DidCommProtocol
                .open(&auditor, &held.bytes, w.registry.as_ref(), NOW)
                .is_ok()
        );
    }
    assert!(
        !readable_by(&w, &auditor)
            .iter()
            .any(|x| x.hash() == m.hash())
    );
    assert!(
        readable_by(&w, &seller)
            .iter()
            .any(|x| x.hash() == m.hash())
    );
    w.settle();
    // The seller quotes to buyer and auditor. The auditor holds the quote until it learns the
    // request's hash, which the quote names; it never reads the request itself.
    let q = w
        .a("seller")
        .member
        .send(
            &id,
            "quote",
            json!({ "price": 1.5, "currency": "EUR" }),
            &["buyer", "auditor"],
        )
        .expect("quote");
    let report = w.send("seller", &q);
    assert_eq!(report.sent.len(), 2);
    assert_eq!(
        report
            .sent
            .iter()
            .find(|c| c.did == auditor.identity.did)
            .map(|c| c.via),
        Some(Via::Relay(TransportKind::DidComm))
    );
    let out = w.settle();
    assert!(
        out["auditor"].contains(&Outcome::Held(q.hash())),
        "{:?}",
        out["auditor"]
    );
    let released = w.a("auditor").member.observe_hidden(&id, &m.hash());
    assert!(journaled(&released, &q.hash()));
    let a = &w.agents["auditor"].member;
    assert!(!a.journal().iter().any(|e| e.message.hash() == m.hash()));
    assert!(
        a.instance(&id)
            .expect("instance")
            .knowledge()
            .contains("Decided")
    );
    assert!(
        !readable_by(&w, &auditor)
            .iter()
            .any(|x| x.hash() == m.hash())
    );
    // The released quote's copy is acknowledged on the next pickup, as a duplicate: the relays
    // then hold nothing more for anyone.
    w.settle();
    assert!(w.mediator.store().held().is_empty());
}

// @tg: verifies:: [[openspec:protocol-spaces#Pairwise Sealed Fan-Out#Any transport carries the envelope]] {level: integration}
#[test]
fn any_transport_carries_the_envelope() {
    // Three seller participants with one DID and one state: one is reached over TSP, one over
    // DIDComm and one over the local channel.
    let mut w = three();
    let id = open_space(&mut w);
    let seller = w.agents["seller"].actor.clone();
    let setup: Vec<Message> = w.agents["seller"]
        .member
        .journal()
        .iter()
        .map(|e| e.message.clone())
        .collect();
    let replica = |w: &World| {
        let mut m = SpaceMember::new(
            Arc::new(ActorSigner(seller.clone())),
            Arc::new(vertep_federation::protocol::ResolverVerifier(
                w.registry.clone() as Arc<dyn DidResolver>,
            )),
        )
        .with_policy(admit(&quote()));
        m.know(quote());
        for msg in &setup {
            if msg.body.sender == seller.identity.did {
                // Its own join: the replica signs the same bytes again.
                let again = m.join(&id).expect("join");
                assert_eq!(again, *msg, "Ed25519 signing is deterministic");
            } else {
                m.receive(msg, 0);
            }
        }
        m
    };
    let mut tsp = replica(&w);
    let mut didcomm = replica(&w);
    let mut local = replica(&w);
    let ok = w
        .a("buyer")
        .member
        .send(
            &id,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .expect("request");
    let early = forged(&w, "buyer", "buyer", &id, "order", &["seller"]);
    let mut unsigned = ok.clone();
    unsigned.signature = "AAAA".into();
    let mut verdicts: Vec<Vec<Vec<Outcome>>> = Vec::new();
    let buyer = w.agents["buyer"].actor.clone();
    let bindings = w.agents["buyer"].member.bindings_for(&ok);
    for (member, kind) in [
        (&mut tsp, Some(TransportKind::Tsp)),
        (&mut didcomm, Some(TransportKind::DidComm)),
        (&mut local, None),
    ] {
        let mut per = Vec::new();
        for msg in [&ok, &early, &unsigned] {
            let out = match kind {
                Some(k) => {
                    let t = SpaceTransport::new(
                        w.registry.clone() as Arc<dyn DidResolver>,
                        w.net.clone() as Arc<dyn RelayNetwork>,
                    )
                    .protocol(w.tsp.clone() as Arc<dyn RelayProtocol>)
                    .protocol(Arc::new(DidCommProtocol))
                    .preference(vec![k]);
                    let r = t.fan_out(&buyer, msg, &bindings);
                    assert_eq!(r.sent[0].via, Via::Relay(k));
                    let (source, proto): (
                        &dyn vertep_federation::PickupSource,
                        &dyn RelayProtocol,
                    ) = match k {
                        TransportKind::Tsp => (w.agents["seller"].inbound.as_ref(), w.tsp.as_ref()),
                        TransportKind::DidComm => (w.mediator.as_ref(), &DidCommProtocol),
                    };
                    member.pickup(&seller, source, proto, w.registry.as_ref(), NOW)
                }
                None => {
                    let ch = LocalChannel::new();
                    ch.host(&seller.identity.did);
                    let t = SpaceTransport::new(
                        w.registry.clone() as Arc<dyn DidResolver>,
                        w.net.clone() as Arc<dyn RelayNetwork>,
                    )
                    .local(ch.clone());
                    let r = t.fan_out(&buyer, msg, &bindings);
                    assert_eq!(r.sent[0].via, Via::Local);
                    member.pickup_local(&ch, 0)
                }
            };
            per.push(out);
        }
        verdicts.push(per);
    }
    assert_eq!(verdicts[0], verdicts[1], "TSP and DIDComm agree");
    assert_eq!(verdicts[0], verdicts[2], "TSP and the local channel agree");
    assert!(journaled(&verdicts[0][0], &ok.hash()));
    assert!(matches!(
        verdicts[0][1].as_slice(),
        [Outcome::Refused {
            refusal: Refusal::Check(Reason::NotEnabledInState),
            ..
        }]
    ));
    assert!(matches!(
        verdicts[0][2].as_slice(),
        [Outcome::Refused {
            refusal: Refusal::Check(Reason::Signature),
            ..
        }]
    ));
}

#[test]
fn a_relay_never_holds_plaintext() {
    let mut w = three();
    let id = open_space(&mut w);
    let m = request(&mut w, &id);
    let item = m.body.payload["item"].as_str().expect("item").to_owned();
    let marker = format!("\"item\":\"{item}\"");
    for (_, bytes) in w.net.posts() {
        assert!(!String::from_utf8_lossy(&bytes).contains(&marker));
    }
    for held in w.mediator.store().held() {
        assert!(!String::from_utf8_lossy(&held.bytes).contains(&marker));
    }
}

// ---------------------------------------------------------------------------------------------
// Hash-Linked History
// ---------------------------------------------------------------------------------------------

// @tg: verifies:: [[openspec:protocol-spaces#Hash-Linked History#Divergence found]] {level: integration}
#[test]
fn divergence_found_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    // The seller's TSP endpoint is down, so the request falls back to the mediator, where it
    // waits for the seller's pickup; comparing heads names it.
    w.net.set_down("https://seller.example/tsp", true);
    let m = w
        .a("buyer")
        .member
        .send(
            &id,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .expect("request");
    let report = w.send("buyer", &m);
    assert_eq!(report.sent[0].via, Via::Relay(TransportKind::DidComm));
    let heads = w.agents["buyer"].member.instance(&id).expect("i").heads();
    let seller = w.agents["seller"].member.instance(&id).expect("i");
    assert_eq!(seller.missing_from(&heads), [m.hash()]);
    w.settle();
    let seller = w.agents["seller"].member.instance(&id).expect("i");
    assert!(seller.missing_from(&heads).is_empty());
}

// @tg: verifies:: [[openspec:protocol-spaces#Hash-Linked History#Hidden message keeps the chain]] {level: integration}
#[test]
fn hidden_message_keeps_the_chain_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    let m = request(&mut w, &id);
    w.settle();
    let q = w
        .a("seller")
        .member
        .send(
            &id,
            "quote",
            json!({ "price": 2, "currency": "USD" }),
            &["buyer", "auditor"],
        )
        .expect("quote");
    w.send("seller", &q);
    w.settle();
    // The seller forwards the auditor's view of the history: the request as its hash only.
    let history = w.agents["seller"]
        .member
        .instance(&id)
        .expect("i")
        .history_for("auditor");
    assert!(
        history
            .iter()
            .any(|e| matches!(e, vertep_kolyada::HistoryEntry::Hidden(h) if *h == m.hash()))
    );
    let late = vertep_kolyada::Instance::from_history(
        quote(),
        &history,
        &w.did("auditor"),
        &vertep_federation::protocol::ResolverVerifier(w.registry.clone() as Arc<dyn DidResolver>),
    )
    .expect("the chain verifies end to end");
    assert!(late.has_applied(&m.hash()) && late.has_applied(&q.hash()));
    assert!(late.knowledge().contains("Decided"));
}

// ---------------------------------------------------------------------------------------------
// Sub-Protocol Invocation
// ---------------------------------------------------------------------------------------------

// @tg: verifies:: [[openspec:protocol-spaces#Sub-Protocol Invocation#Child completes the parent step]] {level: integration}
#[test]
fn child_completes_the_parent_step_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    let m = request(&mut w, &id);
    w.settle();
    let pay = payment(false);
    w.a("seller").member.know(pay.clone());
    w.a("seller").member.set_policy(admit(&pay));
    let child_bindings = BTreeMap::from([
        ("payer".to_owned(), w.did("buyer")),
        ("payee".to_owned(), w.did("seller")),
    ]);
    let genesis = w
        .a("buyer")
        .member
        .open(pay, "payer", &child_bindings, Some((&id, &m.hash())))
        .expect("child genesis");
    let child = genesis.hash();
    w.send("buyer", &genesis);
    w.settle();
    assert_eq!(
        w.agents["seller"]
            .member
            .instance(&child)
            .expect("child")
            .parent(),
        Some(m.hash().as_str())
    );
    let j = w.a("seller").member.join(&child).expect("join");
    w.send("seller", &j);
    w.settle();
    let p = w
        .a("buyer")
        .member
        .send(&child, "pay", json!({}), &["payee"])
        .expect("pay");
    w.send("buyer", &p);
    w.settle();
    let paid = w
        .a("seller")
        .member
        .send(&child, "paid", json!({}), &["payer"])
        .expect("paid");
    w.send("seller", &paid);
    w.settle();
    let done = w
        .a("buyer")
        .member
        .send(&child, control::COMPLETE, json!({}), &["payee"])
        .expect("complete");
    assert_eq!(done.body.protocol, payment(false).id().as_str());
    w.send("buyer", &done);
    w.settle();
    for n in ["buyer", "seller"] {
        assert_eq!(
            w.agents[n].member.instance(&child).expect("child").status(),
            Status::Completed,
            "{n}"
        );
    }
}

// @tg: verifies:: [[openspec:protocol-spaces#Sub-Protocol Invocation#Bindings come from the parent]] {level: integration}
#[test]
fn bindings_come_from_the_parent_over_relays() {
    let mut w = three();
    let id = open_space(&mut w);
    let m = request(&mut w, &id);
    w.settle();
    w.add("stranger", Reach::DidComm);
    let escrow = payment(true);
    w.a("seller").member.know(escrow.clone());
    w.a("seller").member.set_policy(admit(&escrow));
    // A child genesis that binds the bank role to a DID outside the parent: signed by a bound
    // initiator, so only the parent check stands in its way.
    let body = Body {
        protocol: escrow.id().as_str().into(),
        instance: None,
        sender: w.did("buyer"),
        role: "payer".into(),
        seq: 1,
        deps: Vec::new(),
        recipients: vec!["bank".into(), "payee".into()],
        kind: control::GENESIS.into(),
        payload: json!({
            "bindings": { "payer": w.did("buyer"), "payee": w.did("seller"), "bank": w.did("stranger") },
            "parent": m.hash(),
        }),
    };
    let genesis =
        Message::sign(body, &ActorSigner(w.agents["buyer"].actor.clone())).expect("signs");
    let bindings: BTreeMap<String, String> = genesis.body.payload["bindings"]
        .as_object()
        .expect("bindings")
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
        .collect();
    let report = w
        .transport(&[TransportKind::Tsp, TransportKind::DidComm])
        .fan_out(&w.agents["buyer"].actor, &genesis, &bindings);
    assert_eq!(report.sent.len(), 2);
    let out = w.pickup("seller");
    assert!(
        matches!(
            out.as_slice(),
            [Outcome::Refused {
                refusal: Refusal::Check(Reason::BadGenesis(_)),
                ..
            }]
        ),
        "{out:?}"
    );
    assert!(
        w.agents["seller"]
            .member
            .instance(&genesis.hash())
            .is_none()
    );
}
