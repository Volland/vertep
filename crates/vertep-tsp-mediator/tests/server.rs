//! The `vt-mediator` binary over real sockets: started from a configuration file, it serves
//! TSP over HTTP, its DID document, health and metrics, and live sessions over WebSocket.
#![cfg(feature = "server")]

mod common;

use std::io::Write as _;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::{envelope, peer, tsp_service};
use url::Url;
use vertep_federation::tsp::{TspIdentity, TspSession, vid_from_document};
use vertep_federation::tsp_mailbox::TspMailboxClient;
use vertep_federation::{
    Did, DidDocument, DidResolver, InMemoryKeyManager, KeyManager, PickupSource, RelayNetwork,
    ResolverRegistry,
};
use vertep_tsp_mediator::http::{HttpNetwork, WsLiveNetwork};

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn get(url: &str) -> Option<String> {
    ureq::get(url).call().ok()?.into_string().ok()
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

// @tg: verifies:: [[openspec:tsp-mediator#Mediator Operability#Start from configuration]] {level: e2e}
// @tg: verifies:: [[openspec:relay-mailboxes#Live Delivery Over WebSocket#Live TSP session]] {level: e2e}
#[test]
fn the_binary_serves_tsp_its_document_and_live_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let base = format!("http://127.0.0.1:{port}");
    let public = format!("{base}/tsp");
    let config = dir.path().join("mediator.toml");
    let mut f = std::fs::File::create(&config).unwrap();
    write!(
        f,
        r#"
listen = "127.0.0.1:{port}"
public_url = "{public}"
data_dir = "{data}"
did = "peer"
shards = 2

[limits]
max_messages = 50
retention_secs = 3600
"#,
        data = dir.path().join("data").display()
    )
    .unwrap();
    let _server = Running(
        Command::new(env!("CARGO_BIN_EXE_vt-mediator"))
            .args(["serve", "--config"])
            .arg(&config)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("vt-mediator starts"),
    );
    until("health", || {
        get(&format!("{base}/health")).as_deref() == Some("ok")
    });

    // The DID document names the configured endpoint, and the DID resolves to it.
    let doc: DidDocument = DidDocument::from_json(
        &serde_json::from_str(&get(&format!("{base}/did.json")).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        vertep_federation::tsp::tsp_uri(&doc).as_deref(),
        Some(public.as_str())
    );
    assert!(get(&format!("{base}/.well-known/did.json")).is_some());
    let resolver = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let resolved = resolver
        .resolve(&Did::parse(&doc.id).unwrap())
        .unwrap()
        .document;
    assert_eq!(
        vertep_federation::tsp::tsp_uri(&resolved).as_deref(),
        Some(public.as_str())
    );
    assert!(
        dir.path().join("data/identity.json").exists(),
        "identity kept"
    );

    // B registers over HTTP and opens a live session over WebSocket.
    let net = Arc::new(HttpNetwork::default());
    let km: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());
    // B's own endpoint refuses connections: it is offline.
    let b_uri = "http://127.0.0.1:1/tsp";
    let b_id = peer(
        km.as_ref(),
        vec![tsp_service(b_uri, std::slice::from_ref(&doc.id))],
    );
    let b = Arc::new(TspSession::new(
        TspIdentity::new(&b_id, km.clone(), Url::parse(b_uri).unwrap()).unwrap(),
        true,
    ));
    let client = TspMailboxClient::with_live(
        &doc.id,
        net.clone(),
        resolver.clone(),
        Arc::new(WsLiveNetwork),
    )
    .unwrap();
    client.register(b.clone()).expect("registered over HTTP");
    let woken = Arc::new(AtomicUsize::new(0));
    let wk = woken.clone();
    assert!(client.live(
        std::slice::from_ref(&b_id.did),
        Arc::new(move |_: &str| {
            wk.fetch_add(1, Ordering::SeqCst);
        })
    ));

    // A routes a message through the mediator; it is pushed on the live session.
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
    let mediator_vid = vid_from_document(&resolved).unwrap();
    let b_vid = vid_from_document(
        &resolver
            .resolve(&Did::parse(&b_id.did).unwrap())
            .unwrap()
            .document,
    )
    .unwrap();
    let sealed = a
        .seal_routed(&b_vid, &[mediator_vid], &envelope("ws-1"))
        .unwrap();
    net.post(&public, &sealed.bytes).expect("accepted");
    until("pushed", || woken.load(Ordering::SeqCst) > 0);
    let got = client.fetch(std::slice::from_ref(&b_id.did), 10).unwrap();
    assert_eq!(got.len(), 1);
    let mut bytes = got[0].bytes.clone();
    let opened = b
        .open(&mut bytes, &|id: &str| {
            vid_from_document(&resolver.resolve(&Did::parse(id).unwrap()).unwrap().document)
        })
        .unwrap();
    match opened {
        vertep_federation::tsp::TspReceived::Event(m) => {
            assert_eq!(m.envelope.message_id, "ws-1");
            assert_eq!(m.origin.sender.as_deref(), Some(a_id.did.as_str()));
        }
        other => panic!("{other:?}"),
    }
    // Acknowledged over the live session; the mailbox empties.
    client.ack(&[got[0].id.clone()]).unwrap();
    until("acknowledged", || {
        client.status(&b_id.did).map(|s| s.message_count) == Ok(0)
    });
    let metrics = get(&format!("{base}/metrics")).unwrap();
    assert!(metrics.contains("vt_mediator_stored_total 1"), "{metrics}");
    assert!(metrics.contains("vt_mediator_pushed_total 1"), "{metrics}");
}
