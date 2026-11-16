//! Chart role binding (M10, `Chart Role Binding`): two organisations play a quote protocol
//! through a DIDComm mediator. Protocol messages are checked at the grants slot of the inbox
//! and in the send path of the outbox, and the checker state is derived from the journal.
#![cfg(feature = "didcomm")]
#![allow(missing_docs)]

mod world;

use std::sync::Arc;

use serde_json::json;
use vertep_core::datamodel::{Event, Value};
use vertep_core::ir::Chart;
use vertep_federation::didcomm::DidCommProtocol;
use vertep_federation::role_binding::{PROTOCOL_EVENT, SENT_EVENT};
use vertep_federation::{
    ActorIdentity, DidResolver, Grant, KeySigner, LocalActor, OutboundOutcome, Recipients, Refusal,
    RelayProtocol, RoleBinding,
};
use vertep_kolyada::{Body, Message, Protocol, Status};
use world::*;

fn protocol_doc() -> serde_json::Value {
    json!({
        "@context": "https://w3id.org/vertep/kolyada/1.0",
        "type": "Protocol",
        "version": "1.0",
        "roles": ["buyer", "seller"],
        "messages": [
            { "id": "quote.request", "payload": { "fields": [
                { "name": "item", "datatype": "string", "min": 1, "max": 1 },
                { "name": "qty", "datatype": "integer", "min": 1, "max": 1 }
            ] } },
            { "id": "quote", "payload": { "fields": [
                { "name": "price", "datatype": "number", "min": 1, "max": 1 }
            ] } },
            { "id": "order" },
            { "id": "cancel" }
        ],
        "initial": "Idle",
        "final": ["Ordered", "Cancelled"],
        "transitions": [
            { "from": "Idle", "sender": "buyer", "receivers": ["seller"], "message": "quote.request", "to": "Quoting" },
            { "from": "Quoting", "sender": "seller", "receivers": ["buyer"], "message": "quote", "to": "Decided" },
            { "from": "Decided", "sender": "buyer", "receivers": ["seller"], "message": "order", "to": "Ordered" },
            { "from": "Decided", "sender": "buyer", "receivers": ["seller"], "message": "cancel", "to": "Cancelled" }
        ],
        "limits": { "bufferSize": 4, "timeoutMs": 60000 }
    })
}

fn protocol() -> Arc<Protocol> {
    Arc::new(Protocol::parse(protocol_doc().to_string().as_bytes()).expect("the protocol parses"))
}

/// The world's sender chart, declaring the role it plays: `send` sends `_event.data.event` to
/// `_event.data.to` with `_event.data.payload` as data, and everything it receives is journaled.
fn player(role: &str) -> Chart {
    let id = protocol().id().as_str().to_owned();
    let xml = format!(
        r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
               version="1.0" datamodel="ecmascript" initial="idle" vt:plays="{id}#{role}">
              <datamodel><data id="lastid" expr="''"/></datamodel>
              <state id="idle">
                <transition event="send">
                  <send eventexpr="_event.data.event" targetexpr="_event.data.to" idlocation="lastid">
                    <content expr="_event.data.payload"/>
                  </send>
                </transition>
                <transition event="*"/>
              </state>
            </scxml>"##
    );
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(&xml, &opts).expect("chart reads").chart
}

struct Market {
    /// Keeps the network, clock and mediator alive.
    _w: World,
    buyer: Org,
    seller: Org,
    b: ActorIdentity,
    s: ActorIdentity,
    resolutions: Arc<Counting>,
}

fn didcomm() -> Arc<dyn RelayProtocol> {
    Arc::new(DidCommProtocol)
}

/// A buyer and a seller, each bound to its role; the seller's join policy admits any genesis.
/// No inbound grant exists anywhere: protocol messages need none.
fn market() -> Market {
    let w = World::new();
    let m = mediator(&w);
    let resolutions = Counting::new(w.registry.clone());
    let spec = |m: &Arc<vertep_federation::testkit::TestMediator>,
                resolver: Option<Arc<dyn DidResolver>>| OrgSpec {
        protocols: vec![didcomm()],
        bindings: vec![mediator_binding(m, Recipients::Shared)],
        resolver,
        ..OrgSpec::default()
    };
    let mut buyer = w.org(spec(&m, None));
    let mut seller = w.org(spec(&m, Some(resolutions.clone())));
    for (o, role) in [(&mut buyer, "buyer"), (&mut seller, "seller")] {
        o.node
            .register_type(&ty(role), player(role))
            .expect("player chart");
    }
    let rk = vec![m.routing_key()];
    let b = buyer.did_actor("buyer", "b1", vec![didcomm_service(MEDIATOR, &rk)]);
    let s = seller.did_actor("seller", "s1", vec![didcomm_service(MEDIATOR, &rk)]);
    let bind = |o: &Org, did: &str, role: &str| {
        let binding = RoleBinding::new(did, &player(role), [protocol()])
            .expect("binding")
            .with_genesis_policy(Arc::new(|_, _| true));
        assert!(o.fed.bind_roles(binding).expect("bound"));
    };
    bind(&buyer, &b.did, "buyer");
    bind(&seller, &s.did, "seller");
    Market {
        _w: w,
        buyer,
        seller,
        b,
        s,
        resolutions,
    }
}

impl Market {
    fn settle(&mut self) {
        settle(&mut [&mut self.buyer, &mut self.seller]);
    }

    /// The actor `from` (on `org`) sends `event` to `to` with `payload`.
    fn send(org: &Org, from: &str, to: &str, event: &str, payload: Value) {
        org.node
            .send(
                &addr(from),
                ev(
                    "send",
                    obj(&[("to", s(to)), ("event", s(event)), ("payload", payload)]),
                ),
            )
            .expect("send");
    }

    fn roles(org: &Org, did: &str) -> Arc<RoleBinding> {
        org.fed
            .identities()
            .entry(did)
            .and_then(|e| e.roles)
            .expect("bound")
    }

    /// Genesis by the buyer, join by the seller: an open instance on both sides.
    fn open(&mut self) -> String {
        let bindings = obj(&[("buyer", s(&self.b.did)), ("seller", s(&self.s.did))]);
        Self::send(
            &self.buyer,
            &self.b.did,
            &self.s.did,
            "kolyada.genesis",
            obj(&[
                ("protocol", s(protocol().id().as_str())),
                ("role", s("buyer")),
                ("bindings", bindings),
            ]),
        );
        self.settle();
        let id = Self::roles(&self.seller, &self.s.did).instances()[0].clone();
        Self::send(
            &self.seller,
            &self.s.did,
            &self.b.did,
            "kolyada.join",
            obj(&[("instance", s(&id))]),
        );
        self.settle();
        for (o, did) in [(&self.buyer, &self.b.did), (&self.seller, &self.s.did)] {
            let inst = Self::roles(o, did).instance(&id).expect("instance");
            assert_eq!(inst.status(), Status::Open, "{did}");
        }
        id
    }

    fn request(&mut self, id: &str) {
        Self::send(
            &self.buyer,
            &self.b.did,
            &self.s.did,
            "quote.request",
            obj(&[
                ("instance", s(id)),
                (
                    "payload",
                    obj(&[("item", s("widget")), ("qty", Value::Number(3.0))]),
                ),
            ]),
        );
        self.settle();
    }
}

fn data<'e>(e: &'e Event, key: &str) -> Option<&'e Value> {
    match &e.data {
        Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

#[test]
fn chart_sees_an_ordinary_event_with_instance_and_role() {
    let mut m = market();
    let id = m.open();
    m.request(&id);
    let seller = addr(&m.s.did);
    let inputs = inputs(&m.seller.node, &seller);
    let names: Vec<&str> = inputs.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["kolyada.genesis", "send", SENT_EVENT, "quote.request"],
        "genesis, the chart's send of its join, the signed join, the request"
    );
    let req = &inputs[3];
    assert_eq!(req.origin.as_deref(), Some(m.b.did.as_str()));
    assert!(req.originverified);
    assert_eq!(data(req, "instance"), Some(&s(&id)));
    assert_eq!(data(req, "role"), Some(&s("buyer")));
    assert_eq!(data(req, "verdict"), Some(&s("accepted")));
    assert_eq!(
        data(req, "payload"),
        Some(&obj(&[("item", s("widget")), ("qty", Value::Number(3.0))]))
    );
    // No inbound grant exists: the role is the authority.
    assert!(
        m.seller
            .fed
            .inbound_records()
            .iter()
            .all(|r| r.refusal.is_none())
    );
}

// @tg: verifies:: [[openspec:protocol-spaces#Chart Role Binding#Inbound check before the journal]] {level: integration}
#[test]
fn inbound_check_before_the_journal() {
    let mut m = market();
    let id = m.open();
    let seller = addr(&m.s.did);
    let before = inputs(&m.seller.node, &seller).len();
    let records_before = m.seller.fed.inbound_records().len();
    // The buyer signs an `order` before any quote and sends it around its own send-path check,
    // as a plain relay event (which needs an outbound grant on the buyer's side only).
    let inst = Market::roles(&m.buyer, &m.b.did)
        .instance(&id)
        .expect("instance");
    let signer = KeySigner::new(LocalActor {
        identity: m.b.clone(),
        km: m.buyer.km.clone(),
    });
    let forged = Message::sign(
        Body {
            protocol: protocol().id().as_str().into(),
            instance: Some(id.clone()),
            sender: m.b.did.clone(),
            role: "buyer".into(),
            seq: 10,
            deps: inst.heads(),
            recipients: vec!["seller".into()],
            kind: "order".into(),
            payload: json!({}),
        },
        &signer,
    )
    .expect("signed");
    m.buyer
        .fed
        .grant_outbound(Grant::new(m.b.did.as_str(), m.s.did.as_str(), ["*"]));
    Market::send(
        &m.buyer,
        &m.b.did,
        &m.s.did,
        PROTOCOL_EVENT,
        s(&forged.to_json()),
    );
    m.settle();
    assert_eq!(
        inputs(&m.seller.node, &seller).len(),
        before,
        "nothing is appended to the seller's journal"
    );
    let records = m.seller.fed.inbound_records();
    assert_eq!(records.len(), records_before + 1);
    let refusal = records.last().and_then(|r| r.refusal.clone());
    assert!(
        matches!(&refusal, Some(Refusal::Protocol(r)) if r == "not enabled in state"),
        "{refusal:?}"
    );
    // The seller's checker state did not move.
    let after = Market::roles(&m.seller, &m.s.did)
        .instance(&id)
        .expect("instance");
    assert_eq!(after.knowledge().iter().collect::<Vec<_>>(), ["Idle"]);
}

// @tg: verifies:: [[openspec:protocol-spaces#Chart Role Binding#Outbound send refused locally]] {level: integration}
#[test]
fn outbound_send_refused_locally() {
    let mut m = market();
    let id = m.open();
    let sent_before = m.seller.fed.outbound_records().len();
    // The seller quotes before any request: its own projection does not allow it.
    Market::send(
        &m.seller,
        &m.s.did,
        &m.b.did,
        "quote",
        obj(&[
            ("instance", s(&id)),
            ("payload", obj(&[("price", Value::Number(10.0))])),
        ]),
    );
    m.settle();
    let out = m.seller.fed.outbound_records();
    assert_eq!(out.len(), sent_before + 1);
    assert!(
        matches!(&out.last().unwrap().outcome, OutboundOutcome::Failed { reason } if reason.contains("not enabled in state")),
        "{:?}",
        out.last()
    );
    let errors: Vec<Event> = inputs(&m.seller.node, &addr(&m.s.did))
        .into_iter()
        .filter(|e| e.name == "error.communication")
        .collect();
    assert_eq!(errors.len(), 1, "the chart gets error.communication");
    // No message left the seller: the buyer saw nothing after the join.
    assert!(
        !names(&m.buyer.node, &addr(&m.b.did))
            .iter()
            .any(|n| n == "quote")
    );
    // After the request the same send goes through and reaches the buyer.
    m.request(&id);
    Market::send(
        &m.seller,
        &m.s.did,
        &m.b.did,
        "quote",
        obj(&[
            ("instance", s(&id)),
            ("payload", obj(&[("price", Value::Number(10.0))])),
        ]),
    );
    m.settle();
    assert!(
        names(&m.buyer.node, &addr(&m.b.did))
            .iter()
            .any(|n| n == "quote")
    );
}

// @tg: verifies:: [[openspec:protocol-spaces#Chart Role Binding#Replay without re-verification]] {level: integration}
#[test]
fn replay_without_reverification() {
    let mut m = market();
    let id = m.open();
    m.request(&id);
    Market::send(
        &m.seller,
        &m.s.did,
        &m.b.did,
        "quote",
        obj(&[
            ("instance", s(&id)),
            ("payload", obj(&[("price", Value::Number(10.0))])),
        ]),
    );
    m.settle();
    let live = Market::roles(&m.seller, &m.s.did)
        .instance(&id)
        .expect("instance");
    assert_eq!(live.knowledge().iter().collect::<Vec<_>>(), ["Decided"]);

    // A restarted actor gets a fresh binding: its instances come from the journal alone.
    let calls = m.resolutions.calls();
    let fresh = RoleBinding::new(&m.s.did, &player("seller"), [protocol()])
        .expect("binding")
        .with_genesis_policy(Arc::new(|_, _| true));
    assert!(fresh.instances().is_empty());
    assert!(m.seller.fed.bind_roles(fresh).expect("recovered"));
    assert_eq!(
        m.resolutions.calls(),
        calls,
        "no DID is resolved and no signature verified on replay"
    );
    let replayed = Market::roles(&m.seller, &m.s.did)
        .instance(&id)
        .expect("instance derived from the journal");
    assert_eq!(replayed.status(), live.status());
    assert_eq!(replayed.knowledge(), live.knowledge());
    assert_eq!(replayed.heads(), live.heads());
    assert_eq!(replayed.bindings(), live.bindings());

    // The replayed checker keeps working: the buyer orders, the seller accepts it.
    Market::send(
        &m.buyer,
        &m.b.did,
        &m.s.did,
        "order",
        obj(&[("instance", s(&id))]),
    );
    m.settle();
    assert_eq!(
        names(&m.seller.node, &addr(&m.s.did))
            .last()
            .map(String::as_str),
        Some("order")
    );
}

#[test]
fn default_policy_refuses_every_genesis() {
    let mut m = market();
    let strict = RoleBinding::new(&m.s.did, &player("seller"), [protocol()]).expect("binding");
    assert!(m.seller.fed.bind_roles(strict).expect("bound"));
    let bindings = obj(&[("buyer", s(&m.b.did)), ("seller", s(&m.s.did))]);
    Market::send(
        &m.buyer,
        &m.b.did,
        &m.s.did,
        "kolyada.genesis",
        obj(&[
            ("protocol", s(protocol().id().as_str())),
            ("role", s("buyer")),
            ("bindings", bindings),
        ]),
    );
    m.settle();
    assert!(inputs(&m.seller.node, &addr(&m.s.did)).is_empty());
    let refusal = m
        .seller
        .fed
        .inbound_records()
        .last()
        .and_then(|r| r.refusal.clone());
    assert!(matches!(refusal, Some(Refusal::Protocol(r)) if r.contains("join policy")));
}

#[test]
fn service_entry_is_generated_from_vt_plays() {
    let did = "did:example:seller";
    let binding = RoleBinding::new(did, &player("seller"), [protocol()]).expect("binding");
    let mut doc = json!({ "id": did, "service": [
        { "id": format!("{did}#old"), "type": "VertepProtocol", "serviceEndpoint": [] },
        { "id": format!("{did}#didcomm"), "type": "DIDCommMessaging", "serviceEndpoint": "https://m.example" }
    ] });
    binding.publish_into(&mut doc);
    let offered = vertep_kolyada::offer::offered_protocols(&doc);
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].protocol, protocol().id().as_str());
    assert_eq!(offered[0].roles, ["seller"]);
    assert_eq!(doc["service"].as_array().unwrap().len(), 2);
    // A role the protocol does not declare, or a protocol whose document is missing, is refused.
    assert!(RoleBinding::new(did, &player("auditor"), [protocol()]).is_err());
    assert!(RoleBinding::new(did, &player("seller"), []).is_err());
}
