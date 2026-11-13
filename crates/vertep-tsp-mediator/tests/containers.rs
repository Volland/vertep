//! Two `vt-mediator` containers in a route (`docker/compose.yaml`): a sender reaches m1, m1
//! forwards to m2 over the compose network, and m2 holds the message for its registered
//! recipient until it picks it up.
//!
//! Opt-in: runs only with `VERTEP_MEDIATOR_CONTAINERS=1` and a working `docker compose`; it
//! builds the image, starts both mediators, runs the route and removes them. Otherwise it prints
//! why it skipped and passes.
#![cfg(feature = "server")]

mod common;

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{envelope, peer, tsp_service};
use url::Url;
use vertep_federation::tsp::{TspIdentity, TspReceived, TspSession, vid_from_document};
use vertep_federation::tsp_mailbox::TspMailboxClient;
use vertep_federation::{
    Did, DidDocument, DidResolver, InMemoryKeyManager, KeyManager, NetError, RelayNetwork,
    ResolverRegistry,
};
use vertep_tsp_mediator::http::HttpNetwork;

const COMPOSE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docker/compose.yaml");

/// Posts from the host: the mediators' public URLs name compose hosts, mapped to local ports.
struct FromHost(HttpNetwork);

impl RelayNetwork for FromHost {
    fn post(&self, endpoint: &str, body: &[u8]) -> Result<Vec<u8>, NetError> {
        let url = endpoint
            .replace("http://m1:8700", "http://127.0.0.1:18701")
            .replace("http://m2:8700", "http://127.0.0.1:18702");
        self.0.post(&url, body)
    }
}

struct Compose;

impl Drop for Compose {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["compose", "-f", COMPOSE, "down", "-v"])
            .status();
    }
}

fn document(port: u16) -> Option<DidDocument> {
    let text = ureq::get(&format!("http://127.0.0.1:{port}/did.json"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    DidDocument::from_json(&serde_json::from_str(&text).ok()?).ok()
}

#[test]
fn two_mediators_in_a_route() {
    if std::env::var("VERTEP_MEDIATOR_CONTAINERS").as_deref() != Ok("1") {
        eprintln!("skipped: set VERTEP_MEDIATOR_CONTAINERS=1 to run the mediators in Docker");
        return;
    }
    let up = Command::new("docker")
        .args(["compose", "-f", COMPOSE, "up", "-d", "--build", "--wait"])
        .status()
        .expect("docker compose runs");
    let _down = Compose;
    assert!(up.success(), "docker compose up failed");
    let start = Instant::now();
    let (d1, d2) = loop {
        if let (Some(a), Some(b)) = (document(18701), document(18702)) {
            break (a, b);
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "mediators did not come up"
        );
        std::thread::sleep(Duration::from_millis(500));
    };
    let resolver = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let net = Arc::new(FromHost(HttpNetwork::default()));
    let km: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());

    // B is offline and routes through m1 then m2; it registers its mailbox at m2.
    let b_uri = "http://127.0.0.1:1/b";
    let b_id = peer(
        km.as_ref(),
        vec![tsp_service(b_uri, &[d1.id.clone(), d2.id.clone()])],
    );
    let b = Arc::new(TspSession::new(
        TspIdentity::new(&b_id, km.clone(), Url::parse(b_uri).unwrap()).unwrap(),
        true,
    ));
    let client = TspMailboxClient::new(&d2.id, net.clone(), resolver.clone()).unwrap();
    client.register(b.clone()).expect("registered at m2");

    // A sends through m1 and m2.
    let a_id = peer(km.as_ref(), vec![tsp_service("http://127.0.0.1:1/a", &[])]);
    let a = TspSession::new(
        TspIdentity::new(
            &a_id,
            km.clone(),
            Url::parse("http://127.0.0.1:1/a").unwrap(),
        )
        .unwrap(),
        true,
    );
    let vid = |did: &str| {
        vid_from_document(
            &resolver
                .resolve(&Did::parse(did).unwrap())
                .unwrap()
                .document,
        )
    };
    let sealed = a
        .seal_routed(
            &vid(&b_id.did).unwrap(),
            &[vid(&d1.id).unwrap(), vid(&d2.id).unwrap()],
            &envelope("route-1"),
        )
        .unwrap();
    net.post("http://m1:8700/tsp", &sealed.bytes)
        .expect("m1 accepts after m2 stored it");

    let batch = client.deliver(&b_id.did, 10).expect("picked up at m2");
    assert_eq!(batch.messages.len(), 1);
    let mut bytes = batch.messages[0].data.clone();
    match b.open(&mut bytes, &|id: &str| vid(id)).expect("B opens") {
        TspReceived::Event(m) => {
            assert_eq!(m.envelope.message_id, "route-1");
            assert_eq!(m.origin.sender.as_deref(), Some(a_id.did.as_str()));
        }
        other => panic!("{other:?}"),
    }
    client
        .received(&b_id.did, &[batch.messages[0].id.clone()])
        .unwrap();
    assert_eq!(client.status(&b_id.did).unwrap().message_count, 0);
}
