//! Live delivery for DIDComm mailboxes (Message Pickup 3.0 `live-delivery-change`): the
//! mediator pushes deliveries on a live session, the relay mailbox journals them without a
//! poll and acknowledges after the append; a dropped session loses nothing.
#![cfg(feature = "didcomm")]

mod world;

use std::sync::Arc;

use vertep_core::datamodel::Value;
use vertep_federation::didcomm::{DidCommAgent, DidCommProtocol};
use vertep_federation::live::live_uri;
use vertep_federation::pickup::MediatorClient;
use vertep_federation::{
    ActorIdentity, DidMethodChoice, Grant, IdentityConfig, InMemoryKeyManager, KeyManager,
    LocalLiveNetwork, PickupBinding, Recipients, RelayProtocol, SystemClock,
};
use world::*;

struct Live {
    w: World,
    live: Arc<LocalLiveNetwork>,
    m: Arc<vertep_federation::testkit::TestMediator>,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
}

fn setup() -> Live {
    let w = World::new();
    let m = mediator(&w);
    let live = LocalLiveNetwork::new();
    live.serve(&live_uri(MEDIATOR), m.clone());
    let km_b = Arc::new(InMemoryKeyManager::new());
    let b_id = ActorIdentity::provision(
        km_b.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![didcomm_service(MEDIATOR, &[m.routing_key()])],
        }),
    )
    .unwrap();
    let client = Arc::new(
        MediatorClient::new(
            DidCommAgent::new(b_id.clone(), km_b.clone() as Arc<dyn KeyManager>),
            m.did(),
            w.net.clone(),
            w.registry.clone(),
            Arc::new(SystemClock),
        )
        .unwrap()
        .with_live(live.clone()),
    );
    client.request_mediation().unwrap();
    client
        .add_recipients(std::slice::from_ref(&b_id.did))
        .unwrap();
    let didcomm: Arc<dyn RelayProtocol> = Arc::new(DidCommProtocol);
    let a = w.org(OrgSpec {
        protocols: vec![didcomm.clone()],
        ..OrgSpec::default()
    });
    let b = w.org(OrgSpec {
        protocols: vec![didcomm.clone()],
        km: Some(km_b),
        bindings: vec![PickupBinding {
            name: "mediator".into(),
            source: client,
            protocol: didcomm,
            recipients: Recipients::Dedicated(b_id.did.clone()),
        }],
        ..OrgSpec::default()
    });
    b.rehost("receiver", "b1", &b_id);
    let a_id = a.did_actor(
        "sender",
        "a1",
        vec![didcomm_service("http://a.invalid", &[])],
    );
    a.fed
        .grant_outbound(Grant::new(a_id.did.as_str(), b_id.did.as_str(), ["*"]));
    b.fed.grant_inbound(Grant::new(
        a_id.did.as_str(),
        b_id.did.as_str(),
        ["order.*"],
    ));
    Live {
        w,
        live,
        m,
        a,
        b,
        a_id,
        b_id,
    }
}

impl Live {
    fn send(&mut self) {
        self.a
            .node
            .send(
                &addr(&self.a_id.did),
                ev(
                    "send",
                    obj(&[
                        ("to", s(&self.b_id.did)),
                        ("event", s("order.placed")),
                        ("payload", obj(&[("n", Value::Number(1.0))])),
                    ]),
                ),
            )
            .unwrap();
        settle(&mut [&mut self.a]);
    }
}

#[test]
fn didcomm_live_delivery_reaches_the_journal_without_a_poll() {
    let mut l = setup();
    assert_eq!(l.b.fed.go_live(), vec!["mediator".to_owned()]);
    let before = l.w.net.posts_to(MEDIATOR);
    l.send();
    l.b.node.run_until_idle();
    let got = names(&l.b.node, &addr(&l.b_id.did));
    assert_eq!(got, vec!["order.placed"]);
    assert!(
        l.m.store().held().is_empty(),
        "acknowledged after the append"
    );
    // Over HTTP only A's forward and B's `messages-received` reached the mediator: no
    // `delivery-request` poll.
    assert_eq!(l.w.net.posts_to(MEDIATOR), before + 2);
}

// @tg: verifies:: [[openspec:relay-mailboxes#Live Delivery Over WebSocket#Session reconnects]] {level: e2e}
#[test]
fn a_dropped_didcomm_session_redelivers_once() {
    let mut l = setup();
    l.b.fed.go_live();
    l.send();
    // Pushed into B's mailbox; the session drops before B runs.
    l.live.drop_connections(&live_uri(MEDIATOR));
    assert_eq!(l.m.store().held().len(), 1, "still at the mediator");
    l.b.fed.go_live();
    settle(&mut [&mut l.b]);
    assert_eq!(names(&l.b.node, &addr(&l.b_id.did)), vec!["order.placed"]);
    assert!(l.m.store().held().is_empty());
}
