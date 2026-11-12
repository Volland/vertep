//! Runtime nodes with TSP actors whose mailboxes live at a `vt-mediator`: the relay-mailboxes
//! client side (`TSP Mediator Mailbox`, `Live Delivery Over WebSocket`), on the federation test
//! world of two organisations.

// The federation test world, shared by path; its DIDComm-only helpers name a feature this
// crate does not have.
#[allow(unexpected_cfgs)]
#[path = "../../vertep-federation/tests/world/mod.rs"]
mod world;

use std::sync::Arc;

use vertep_federation::live::live_uri;
use vertep_federation::tsp::TspProtocol;
use vertep_federation::tsp_mailbox::TspMailboxClient;
use vertep_federation::{
    ActorIdentity, Grant, KeyManager, LocalActor, LocalLiveNetwork, OutboundOutcome, PickupBinding,
    Recipients, RelayProtocol, RuntimeClock,
};
use vertep_store::MemoryStore;
use vertep_tsp_mediator::{MailboxStore, Mediator, MediatorOptions};
use world::*;

const MEDIATOR_TSP: &str = "https://mediator.example/tsp";
const A_TSP: &str = "https://a.example/tsp";
/// B's own endpoint: nothing serves it, B is reachable only through its mediator.
const B_TSP: &str = "https://b.example/tsp";

struct Setup {
    w: World,
    live: Arc<LocalLiveNetwork>,
    m: Arc<Mediator>,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
    b_store: Arc<dyn vertep_runtime::Storage>,
    b_km: Arc<vertep_federation::InMemoryKeyManager>,
}

fn mediator(w: &World, live: &LocalLiveNetwork) -> Arc<Mediator> {
    let km: Arc<dyn KeyManager> = Arc::new(vertep_federation::InMemoryKeyManager::new());
    let identity = ActorIdentity::provision(
        km.as_ref(),
        &vertep_federation::IdentityConfig::new(vertep_federation::DidMethodChoice::Peer {
            services: vec![tsp_service(MEDIATOR_TSP, &[])],
        }),
    )
    .unwrap();
    let m = Mediator::new(
        identity,
        km,
        w.registry.clone(),
        w.net.clone(),
        MailboxStore::open(Arc::new(MemoryStore::new())).unwrap(),
        Arc::new(RuntimeClock(Arc::new(w.clock.clone()))),
        MediatorOptions::default(),
    )
    .unwrap();
    w.net.serve(MEDIATOR_TSP, m.clone());
    live.serve(&live_uri(MEDIATOR_TSP), m.clone());
    m
}

/// B's organisation picks up from the mediator through `client`.
fn org_b(
    w: &World,
    client: &Arc<TspMailboxClient>,
    store: Arc<dyn vertep_runtime::Storage>,
    km: Arc<vertep_federation::InMemoryKeyManager>,
) -> (Org, Arc<TspProtocol>) {
    let b_tsp = Arc::new(TspProtocol::new(true));
    let org = w.org(OrgSpec {
        protocols: vec![b_tsp.clone() as Arc<dyn RelayProtocol>],
        bindings: vec![PickupBinding {
            name: "vt-mediator".into(),
            source: client.clone(),
            protocol: b_tsp.clone(),
            recipients: Recipients::Shared,
        }],
        store: Some(store),
        km: Some(km),
        ..OrgSpec::default()
    });
    (org, b_tsp)
}

fn setup() -> Setup {
    let w = World::new();
    let live = LocalLiveNetwork::new();
    let m = mediator(&w, &live);
    let client =
        TspMailboxClient::with_live(m.did(), w.net.clone(), w.registry.clone(), live.clone())
            .unwrap();
    let a_tsp = Arc::new(TspProtocol::new(true));
    let a = w.org(OrgSpec {
        protocols: vec![a_tsp as Arc<dyn RelayProtocol>],
        ..OrgSpec::default()
    });
    let b_store: Arc<dyn vertep_runtime::Storage> = Arc::new(MemoryStore::new());
    let b_km = Arc::new(vertep_federation::InMemoryKeyManager::new());
    let (b, b_tsp) = org_b(&w, &client, b_store.clone(), b_km.clone());
    let a_id = a.did_actor("sender", "a1", vec![tsp_service(A_TSP, &[])]);
    let b_id = b.did_actor(
        "receiver",
        "b1",
        vec![tsp_service(B_TSP, &[m.did().to_owned()])],
    );
    let session = b_tsp
        .session(&LocalActor {
            identity: b_id.clone(),
            km: b_km.clone() as Arc<dyn KeyManager>,
        })
        .unwrap();
    client
        .register(session)
        .expect("B registers with the mediator");
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed.grant_inbound(Grant::new(
        a_id.did.as_str(),
        b_id.did.as_str(),
        ["order.*"],
    ));
    Setup {
        w,
        live,
        m,
        a,
        b,
        a_id,
        b_id,
        b_store,
        b_km,
    }
}

impl Setup {
    /// A sends `order.placed` to B; only A's node runs.
    fn send(&mut self) {
        self.a
            .node
            .send(
                &addr(&self.a_id.did),
                ev("go", obj(&[("to", s(&self.b_id.did))])),
            )
            .unwrap();
        for _ in 0..5 {
            self.a.node.run_until_idle();
            self.a.fed.pump_outbox();
        }
    }

    fn held(&self) -> usize {
        self.m.store().held(&self.b_id.did, 100).len()
    }

    fn b_names(&self) -> Vec<String> {
        names(&self.b.node, &addr(&self.b_id.did))
    }
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Mediator Mailbox#Message waits for a deactivated actor]] {level: e2e}
#[test]
fn a_message_waits_at_the_mediator_while_the_actor_is_deactivated() {
    let mut s = setup();
    s.send();
    assert!(
        s.a.fed
            .outbound_records()
            .iter()
            .any(|r| matches!(r.outcome, OutboundOutcome::Sent { .. })),
        "A's send completed: {:?}",
        s.a.fed.outbound_records()
    );
    assert_eq!(s.held(), 1, "held at the mediator");
    assert!(s.b_names().is_empty(), "B never ran");
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Mediator Mailbox#Pickup activates and acknowledges after the append]] {level: e2e}
#[test]
fn pickup_activates_the_actor_and_acknowledges_after_the_append() {
    let mut s = setup();
    s.send();
    let report = s.b.fed.pickup();
    assert_eq!(report.delivered.values().sum::<usize>(), 1);
    assert_eq!(s.held(), 1, "not acknowledged before the journal append");
    assert!(s.b_names().is_empty());
    s.b.node.run_until_idle();
    let got = inputs(&s.b.node, &addr(&s.b_id.did));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, "order.placed");
    assert_eq!(got[0].origin.as_deref(), Some(s.a_id.did.as_str()));
    assert_eq!(s.held(), 0, "acknowledged after the append");
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Mediator Mailbox#Crash between append and acknowledgement]] {level: e2e}
#[test]
fn a_crash_between_append_and_acknowledgement_does_not_process_twice() {
    let mut s = setup();
    s.send();
    s.b.fed.pickup();
    // The acknowledgement is lost: the mediator is unreachable when B acknowledges.
    s.w.net.set_down(MEDIATOR_TSP, true);
    s.b.node.run_until_idle();
    assert_eq!(s.b_names(), vec!["order.placed"], "journaled");
    s.w.net.set_down(MEDIATOR_TSP, false);
    assert_eq!(s.held(), 1, "still at the mediator");
    // B's node crashes and restarts on the same storage and keys.
    let (store, km) = (s.b_store.clone(), s.b_km.clone());
    let client = TspMailboxClient::with_live(
        s.m.did(),
        s.w.net.clone(),
        s.w.registry.clone(),
        s.live.clone(),
    )
    .unwrap();
    let (b2, b2_tsp) = org_b(&s.w, &client, store, km.clone());
    b2.rehost("receiver", "b1", &s.b_id);
    let session = b2_tsp
        .session(&LocalActor {
            identity: s.b_id.clone(),
            km: km as Arc<dyn KeyManager>,
        })
        .unwrap();
    client.register(session).unwrap();
    s.b = b2;
    settle(&mut [&mut s.b]);
    assert_eq!(s.b_names(), vec!["order.placed"], "processed once");
    assert_eq!(s.held(), 0, "redelivered, recognised and acknowledged");
}

#[test]
fn live_delivery_reaches_the_journal_without_a_poll() {
    let mut s = setup();
    assert_eq!(s.b.fed.go_live(), vec!["vt-mediator".to_owned()]);
    let posts = s.w.net.posts_to(MEDIATOR_TSP);
    s.send();
    // The message was pushed on the live session and placed in B's mailbox as it arrived.
    s.b.node.run_until_idle();
    assert_eq!(s.b_names(), vec!["order.placed"]);
    assert_eq!(s.held(), 0, "acknowledged over the session");
    // Only A's post reached the mediator over HTTP: nothing from B.
    assert_eq!(s.w.net.posts_to(MEDIATOR_TSP), posts + 1);
}

// @tg: verifies:: [[openspec:relay-mailboxes#Live Delivery Over WebSocket#Session reconnects]] {level: e2e}
#[test]
fn a_reconnected_session_redelivers_without_duplicates() {
    let mut s = setup();
    s.b.fed.go_live();
    s.send();
    // Pushed and placed in B's mailbox, but the session drops before B runs and acknowledges.
    s.live.drop_connections(&live_uri(MEDIATOR_TSP));
    assert_eq!(s.held(), 1);
    // Reconnect: the unacknowledged message is pushed again.
    s.b.fed.go_live();
    settle(&mut [&mut s.b]);
    assert_eq!(s.b_names(), vec!["order.placed"], "once");
    assert_eq!(s.held(), 0);
}
