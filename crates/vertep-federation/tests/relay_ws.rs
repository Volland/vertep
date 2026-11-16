//! WebSocket live delivery for relay mailboxes (M13 design D5): the test mediator's live session
//! on a real socket, a DIDComm mailbox client on `WsLiveNetwork`, and a `PickupLoop` that keeps
//! the inbox live and falls back to polling. The receiving node is only run here, never pumped,
//! so a message reaches its journal through the socket or the loop's polling and nothing else.
#![cfg(all(feature = "didcomm", feature = "ws"))]

mod world;

use std::sync::Arc;
use std::time::{Duration, Instant};

use vertep_core::datamodel::Value;
use vertep_federation::didcomm::{DidCommAgent, DidCommProtocol};
use vertep_federation::pickup::MediatorClient;
use vertep_federation::testkit::TestMediator;
use vertep_federation::ws::{WsLiveNetwork, WsLiveServer};
use vertep_federation::{
    ActorIdentity, DidMethodChoice, Grant, IdentityConfig, InMemoryKeyManager, KeyManager,
    PickupBinding, PickupLoop, PickupLoopConfig, PickupMode, Recipients, RelayProtocol,
    SystemClock,
};
use world::*;

struct Ws {
    w: World,
    server: WsLiveServer,
    endpoint: String,
    m: Arc<TestMediator>,
    a: Org,
    b: Org,
    a_id: ActorIdentity,
    b_id: ActorIdentity,
}

/// A at org 1 sends to B at org 2, whose mailbox is at a test mediator. Posts to the mediator
/// stay in-process; its live sessions are served on a socket at the live URI of its endpoint.
fn setup() -> Ws {
    let w = World::new();
    let server = WsLiveServer::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("{}/didcomm", server.http_base());
    let m = TestMediator::new(
        Arc::new(InMemoryKeyManager::new()),
        &endpoint,
        w.net.clone(),
        w.registry.clone(),
    )
    .unwrap();
    w.net.serve(&endpoint, m.clone());
    server.serve("/didcomm/live", m.clone());
    let km_b = Arc::new(InMemoryKeyManager::new());
    let b_id = ActorIdentity::provision(
        km_b.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![didcomm_service(&endpoint, &[m.routing_key()])],
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
        .with_live(Arc::new(WsLiveNetwork::new(Duration::from_secs(5)))),
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
    let mut b = w.org(OrgSpec {
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
    // The node's first round recovers its mailboxes, which polls once; done here, so what the
    // tests see is the pickup loop and the socket only.
    b.node.run_until_idle();
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
    Ws {
        w,
        server,
        endpoint,
        m,
        a,
        b,
        a_id,
        b_id,
    }
}

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out waiting for {what}");
}

impl Ws {
    /// A sends `event` to B; returns once A's forward reached the mediator.
    fn send(&mut self, event: &str) {
        self.a
            .node
            .send(
                &addr(&self.a_id.did),
                ev(
                    "send",
                    obj(&[
                        ("to", s(&self.b_id.did)),
                        ("event", s(event)),
                        ("payload", obj(&[("n", Value::Number(1.0))])),
                    ]),
                ),
            )
            .unwrap();
        settle(&mut [&mut self.a]);
    }

    fn b_names(&self) -> Vec<String> {
        names(&self.b.node, &addr(&self.b_id.did))
    }

    /// Runs B's node (never its pickup) until `event` is journaled; returns how long it took.
    fn deliver(&mut self, event: &str) -> Duration {
        let t0 = Instant::now();
        let deadline = t0 + Duration::from_secs(10);
        while Instant::now() < deadline {
            self.b.node.run_until_idle();
            if self.b_names().iter().any(|n| n == event) {
                return t0.elapsed();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("{event} not delivered; journal has {:?}", self.b_names());
    }

    fn held(&self) -> usize {
        self.m.store().held().len()
    }

    fn pickup(&self, config: PickupLoopConfig) -> PickupLoop {
        PickupLoop::spawn(self.b.fed.inbox().clone(), config).unwrap()
    }
}

fn config(poll_ms: u64, reconnect_ms: u64) -> PickupLoopConfig {
    PickupLoopConfig {
        poll_interval: Duration::from_millis(poll_ms),
        live_poll_interval: None,
        reconnect_min: Duration::from_millis(reconnect_ms),
        reconnect_max: Duration::from_millis(reconnect_ms * 4),
        check_interval: Duration::from_millis(10),
    }
}

// @tg: verifies:: [[openspec:relay-mailboxes#WebSocket Live Delivery#Live delivery]] {level: integration}
#[test]
fn live_delivery_over_websocket_beats_the_poll_interval() {
    let mut l = setup();
    let poll = Duration::from_secs(5);
    let pl = l.pickup(config(5_000, 200));
    wait_for("the live session", || {
        pl.mode("mediator") == Some(PickupMode::Live)
    });
    assert_eq!(l.server.open_connections(), 1);
    let posts = l.w.net.posts_to(&l.endpoint);
    l.send("order.m1");
    let latency = l.deliver("order.m1");
    eprintln!("live delivery over WebSocket: {latency:?} (poll interval {poll:?})");
    assert!(
        latency < poll / 10,
        "live delivery took {latency:?}, poll interval {poll:?}"
    );
    // Acknowledged on the socket after the append: the mediator lets go of the copy.
    wait_for("the acknowledgement", || l.held() == 0);
    // Over HTTP only A's forward reached the mediator: no delivery-request, no
    // messages-received. The loop polled once, when the session opened.
    assert_eq!(l.w.net.posts_to(&l.endpoint), posts + 1);
    assert_eq!(pl.status()[0].polls, 1);
    assert_eq!(l.b_names(), vec!["order.m1"]);
}

// @tg: verifies:: [[openspec:relay-mailboxes#WebSocket Live Delivery#No double delivery]] {level: integration}
#[test]
fn a_message_acknowledged_on_the_socket_is_not_delivered_again_by_a_poll() {
    let mut l = setup();
    let pl = l.pickup(config(5_000, 200));
    wait_for("the live session", || {
        pl.mode("mediator") == Some(PickupMode::Live)
    });
    l.send("order.m1");
    l.deliver("order.m1");
    wait_for("the acknowledgement", || l.held() == 0);
    let report = l.b.fed.inbox().pickup_at(0);
    assert!(report.delivered.is_empty(), "{report:?}");
    l.b.node.run_until_idle();
    assert_eq!(l.b_names(), vec!["order.m1"]);
}

// @tg: verifies:: [[openspec:relay-mailboxes#WebSocket Live Delivery#Fallback to polling]] {level: integration}
#[test]
fn a_refused_socket_falls_back_to_polling() {
    let mut l = setup();
    l.server.set_refusing(true);
    let poll = Duration::from_millis(100);
    let pl = l.pickup(config(100, 200));
    wait_for("a failed connect", || pl.status()[0].failed_connects >= 1);
    assert_eq!(pl.mode("mediator"), Some(PickupMode::Polling));
    l.send("order.m1");
    let latency = l.deliver("order.m1");
    eprintln!("delivery by polling: {latency:?} (poll interval {poll:?})");
    assert!(pl.status()[0].polls >= 1);
    assert_eq!(pl.status()[0].connects, 0);
    // Acknowledged over HTTP, since there is no socket.
    wait_for("the acknowledgement", || l.held() == 0);
    assert_eq!(l.b_names(), vec!["order.m1"]);
}

// @tg: verifies:: [[openspec:relay-mailboxes#WebSocket Live Delivery#Fallback to polling]] {level: integration}
// @tg: verifies:: [[openspec:relay-mailboxes#WebSocket Live Delivery#No double delivery]] {level: integration}
#[test]
fn a_dropped_socket_loses_nothing_and_processes_nothing_twice() {
    let mut l = setup();
    let pl = l.pickup(config(150, 300));
    wait_for("the live session", || {
        pl.mode("mediator") == Some(PickupMode::Live)
    });

    // m1 live.
    l.send("order.m1");
    l.deliver("order.m1");
    wait_for("m1 acknowledged", || l.held() == 0);

    // m2 is pushed into B's mailbox, then the socket drops before B runs and acknowledges.
    l.send("order.m2");
    wait_for("m2 pushed", || {
        l.b.fed
            .inbound_records()
            .iter()
            .any(|r| r.event.as_deref() == Some("order.m2"))
    });
    assert_eq!(l.held(), 1, "m2 is still at the mediator");
    l.server.set_refusing(true);
    l.server.drop_connections();
    wait_for("the fallback", || {
        pl.mode("mediator") == Some(PickupMode::Polling)
    });
    assert_eq!(pl.status()[0].drops, 1);

    // m3 arrives while there is no socket: polling delivers it.
    l.send("order.m3");
    l.deliver("order.m3");
    l.deliver("order.m2");
    wait_for("m2 and m3 acknowledged over HTTP", || l.held() == 0);

    // The socket comes back; the session resumes and m4 is live again.
    l.server.set_refusing(false);
    wait_for("the reconnect", || pl.status()[0].connects == 2);
    l.send("order.m4");
    l.deliver("order.m4");
    wait_for("m4 acknowledged", || l.held() == 0);

    // A last poll finds nothing, and the journal has every message exactly once.
    assert!(l.b.fed.inbox().pickup_at(0).delivered.is_empty());
    l.b.node.run_until_idle();
    let mut got = l.b_names();
    got.sort();
    assert_eq!(got, vec!["order.m1", "order.m2", "order.m3", "order.m4"]);
}
