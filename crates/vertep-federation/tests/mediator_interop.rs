//! Task 10.1: interop with a third-party DIDComm v2 mediator, the Hyperledger Identus mediator
//! (`ghcr.io/hyperledger/identus-mediator:1.0.0`, Scala, DIDComm v2 with Mediator Coordination
//! 2.0 and Message Pickup 3.0), run in Docker with its MongoDB.
//!
//! The test is opt-in: it runs only with `VERTEP_MEDIATOR_INTEROP=1` and a working `docker`
//! (it pulls the pinned images, starts both containers on a private network, and removes them
//! afterwards), or with `VERTEP_MEDIATOR_URL` pointing at an already running mediator (its
//! `GET /did` must answer the mediator DID). Otherwise it prints why it skipped and passes.
//!
//! Over plain HTTP on localhost it checks the whole wire path: mediation and keylist update,
//! a forward from another actor, `status-request`, `delivery-request`, opening the delivered
//! message with its verified origin, and `messages-received`; then the same through two runtime
//! nodes, where the receiver's relay mailbox picks up from the mediator and acknowledges only
//! after the journal append.
#![cfg(feature = "didcomm")]

mod world;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::didcomm::{DidCommAgent, DidCommProtocol, DidCommReceived};
use vertep_federation::pickup::MediatorClient;
use vertep_federation::{
    ActorIdentity, DidDirectory, DidMethodChoice, Grant, IdentityConfig, InMemoryKeyManager,
    KeyManager, NetError, PickupBinding, Recipients, RelayEnvelope, RelayNetwork, RelayProtocol,
    SystemClock, Timestamp,
};
use world::*;

const MEDIATOR_IMAGE: &str = "ghcr.io/hyperledger/identus-mediator:1.0.0";
const MONGO_IMAGE: &str = "mongo:6.0.19";

/// The database user and collections the mediator expects (its repository's `initdb.js`).
const INITDB: &str = r#"db = db.getSiblingDB("mediator");
db.createUser({ user: "admin", pwd: "admin", roles: [{ role: "readWrite", db: "mediator" }] });
db.createCollection("user.account");
db.createCollection("messages");
db.createCollection("messages.outbound");
db.getCollection("user.account").createIndex({ did: 1 }, { unique: true });
"#;

// ---- a minimal HTTP/1.1 client over std (localhost, no TLS) ------------------------------------

fn split_url(url: &str) -> Result<(String, String), NetError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| NetError::Refused(format!("{url}: only http:// is supported here")))?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    Ok((host.to_owned(), path.to_owned()))
}

fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(eol) = body.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let size = std::str::from_utf8(&body[..eol])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next().unwrap_or("").trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 {
            return out;
        }
        let start = eol + 2;
        let end = (start + size).min(body.len());
        out.extend_from_slice(&body[start..end]);
        body = body.get(end + 2..).unwrap_or_default();
    }
}

fn http(method: &str, url: &str, body: &[u8]) -> Result<(u16, Vec<u8>), NetError> {
    let (host, path) = split_url(url)?;
    let mut s =
        TcpStream::connect(&host).map_err(|e| NetError::Unreachable(format!("{url}: {e}")))?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(60)));
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/didcomm-encrypted+json\r\nAccept: application/didcomm-encrypted+json, */*\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())
        .and_then(|()| s.write_all(body))
        .map_err(|e| NetError::Unreachable(format!("{url}: {e}")))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| NetError::Unreachable(format!("{url}: {e}")))?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| NetError::Unreachable(format!("{url}: no HTTP response")))?;
    let headers = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let status: u16 = headers
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let mut payload = raw[split + 4..].to_vec();
    if headers.contains("transfer-encoding: chunked") {
        payload = dechunk(&payload);
    }
    Ok((status, payload))
}

/// Posts DIDComm messages over HTTP: the network a deployment plugs in at [`RelayNetwork`].
struct Http;

impl RelayNetwork for Http {
    fn post(&self, endpoint: &str, body: &[u8]) -> Result<Vec<u8>, NetError> {
        let (status, payload) = http("POST", endpoint, body)?;
        match status {
            200..=299 => Ok(payload),
            404 | 410 => Err(NetError::Gone(endpoint.to_owned())),
            _ => Err(NetError::Refused(format!(
                "{endpoint}: HTTP {status}: {}",
                String::from_utf8_lossy(&payload)
            ))),
        }
    }
}

// ---- the mediator in Docker ----------------------------------------------------------------

fn docker(args: &[&str]) -> Result<String, String> {
    let out = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| format!("docker: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!(
            "docker {}: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// The containers and network of one run, removed on drop.
struct Containers {
    names: Vec<String>,
    network: String,
    _dir: tempfile::TempDir,
}

impl Drop for Containers {
    fn drop(&mut self) {
        for n in &self.names {
            let _ = docker(&["rm", "-f", n]);
        }
        let _ = docker(&["network", "rm", &self.network]);
    }
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("rng");
    b
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("free port")
}

/// Starts MongoDB and the mediator with fresh keys and returns them with the mediator's URL.
fn start_mediator() -> Result<(Containers, String), String> {
    docker(&["info", "--format", "{{.ServerVersion}}"])?;
    let tag = format!("vertep-it-{}", std::process::id());
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let init = dir.path().join("initdb.js");
    std::fs::write(&init, INITDB).map_err(|e| e.to_string())?;
    let mut c = Containers {
        names: Vec::new(),
        network: tag.clone(),
        _dir: dir,
    };
    docker(&["network", "create", &tag])?;
    let mongo = format!("{tag}-mongo");
    let mount = format!(
        "{}:/docker-entrypoint-initdb.d/initdb.js:ro",
        init.display()
    );
    docker(&[
        "run",
        "-d",
        "--name",
        &mongo,
        "--network",
        &tag,
        "-e",
        "MONGO_INITDB_ROOT_USERNAME=root",
        "-e",
        "MONGO_INITDB_ROOT_PASSWORD=root",
        "-e",
        "MONGO_INITDB_DATABASE=mediator",
        "-v",
        &mount,
        MONGO_IMAGE,
    ])?;
    c.names.push(mongo.clone());

    let x = x25519_dalek::StaticSecret::from(random32());
    let x_pub = x25519_dalek::PublicKey::from(&x);
    let ed = ed25519_dalek::SigningKey::from_bytes(&random32());
    let port = free_port();
    let url = format!("http://localhost:{port}");
    let mediator = format!("{tag}-mediator");
    let envs = [
        format!("KEY_AGREEMENT_D={}", b64(x.as_bytes())),
        format!("KEY_AGREEMENT_X={}", b64(x_pub.as_bytes())),
        format!("KEY_AUTHENTICATION_D={}", b64(ed.as_bytes())),
        format!(
            "KEY_AUTHENTICATION_X={}",
            b64(ed.verifying_key().as_bytes())
        ),
        format!("SERVICE_ENDPOINTS={url}"),
        format!("MONGODB_HOST={mongo}"),
        "MONGODB_USER=admin".to_owned(),
        "MONGODB_PASSWORD=admin".to_owned(),
        "MONGODB_DB_NAME=mediator".to_owned(),
    ];
    let publish = format!("127.0.0.1:{port}:8080");
    let mut args = vec![
        "run",
        "-d",
        "--name",
        &mediator,
        "--network",
        &tag,
        "--platform",
        "linux/amd64",
        "-p",
        &publish,
    ];
    for e in &envs {
        args.push("-e");
        args.push(e);
    }
    args.push(MEDIATOR_IMAGE);
    docker(&args)?;
    c.names.push(mediator);
    Ok((c, url))
}

/// The mediator's DID from `GET /did`, waiting up to three minutes for it to come up (the image
/// is amd64-only and may run emulated).
fn mediator_did(url: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Ok((200, body)) = http("GET", &format!("{url}/did"), b"") {
            let did = String::from_utf8_lossy(&body).trim().to_owned();
            if did.starts_with("did:") {
                return Ok(did);
            }
        }
        if Instant::now() > deadline {
            return Err(format!("{url}/did did not answer a DID in time"));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ---- the test ------------------------------------------------------------------------------

fn peer(km: &Arc<InMemoryKeyManager>, uri: &str, routing_keys: &[String]) -> ActorIdentity {
    ActorIdentity::provision(
        km.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![didcomm_service(uri, routing_keys)],
        }),
    )
    .expect("identity")
}

/// The network, the resolvers and the mediator DID a run talks to.
struct Setup<'a> {
    w: &'a World,
    net: Arc<dyn RelayNetwork>,
    med: &'a str,
}

fn client(id: &ActorIdentity, km: &Arc<InMemoryKeyManager>, x: &Setup<'_>) -> MediatorClient {
    MediatorClient::new(
        DidCommAgent::new(id.clone(), km.clone() as Arc<dyn KeyManager>),
        x.med,
        x.net.clone(),
        x.w.registry.clone(),
        Arc::new(SystemClock),
    )
    .expect("mediator resolves")
}

/// Mediation, forward, Pickup 3.0 and acknowledgement at the wire level.
fn wire_protocol(x: &Setup<'_>) {
    let w = x.w;
    let km = Arc::new(InMemoryKeyManager::new());
    let probe = client(&peer(&km, "http://unused.invalid", &[]), &km, x);
    let url = probe.endpoint().to_owned();
    let rk = vec![probe.routing_key().expect("mediator key-agreement key")];

    let alice = peer(&km, "http://alice.invalid", &[]);
    let bob = peer(&km, &url, &rk);
    let bob_client = client(&bob, &km, x);
    let routing = bob_client.request_mediation().expect("mediate-grant");
    assert!(!routing.is_empty(), "the grant names a routing DID");
    bob_client
        .add_recipients(std::slice::from_ref(&bob.did))
        .expect("keylist-update");
    assert_eq!(bob_client.status(Some(&bob.did)).expect("status"), 0);

    // Alice sends an event to Bob through the mediator: authcrypt to Bob, forward to the mediator.
    let bob_doc = bob.document(km.as_ref()).expect("doc");
    let route = DidDirectory::new(w.registry.clone())
        .resolve(&bob.did)
        .expect("resolves")
        .routes[0]
        .clone();
    let mut e = Event::named("order.placed", EventType::External);
    e.sendid = Some("s-interop".into());
    e.data = Value::Object(vec![("amount".into(), Value::Number(42.0))]);
    let out = DidCommAgent::new(alice.clone(), km.clone() as Arc<dyn KeyManager>)
        .seal_event(
            &bob_doc,
            &route,
            &RelayEnvelope::from_event("m-interop-1", &e),
            false,
            w.registry.as_ref(),
        )
        .expect("sealed");
    assert_eq!(out.endpoint, url);
    x.net
        .post(&out.endpoint, out.message.as_bytes())
        .expect("the mediator accepts the forward");

    assert_eq!(bob_client.status(Some(&bob.did)).expect("status"), 1);
    let batch = bob_client.delivery(Some(&bob.did), 10).expect("delivery");
    assert_eq!(batch.len(), 1);
    // A delivery is not an acknowledgement: the copy is still held.
    assert_eq!(bob_client.status(Some(&bob.did)).expect("status"), 1);
    let jwe = std::str::from_utf8(&batch[0].bytes).expect("utf-8");
    let opened = match DidCommAgent::new(bob.clone(), km.clone() as Arc<dyn KeyManager>)
        .open(jwe, w.registry.as_ref(), Timestamp::now())
        .expect("Bob opens what the mediator stored")
    {
        DidCommReceived::Event(m) => *m,
        other => panic!("expected an event, got {other:?}"),
    };
    assert!(opened.origin.verified);
    assert_eq!(opened.origin.sender.as_deref(), Some(alice.did.as_str()));
    let mut ev = opened.envelope.to_event();
    opened.origin.apply(&mut ev);
    assert_eq!(ev.name, "order.placed");
    assert!(ev.originverified);
    assert_eq!(ev.origin.as_deref(), Some(alice.did.as_str()));
    let left = bob_client
        .messages_received(&[batch[0].id.clone()])
        .expect("messages-received");
    assert!(matches!(left, None | Some(0)), "{left:?}");
    assert_eq!(bob_client.status(Some(&bob.did)).expect("status"), 0);
    assert!(
        bob_client
            .delivery(Some(&bob.did), 10)
            .expect("delivery")
            .is_empty()
    );

    // Another DID cannot pick up Bob's messages.
    let carol = peer(&km, "http://carol.invalid", &[]);
    let carol_client = client(&carol, &km, x);
    carol_client.request_mediation().expect("mediate-grant");
    assert!(
        carol_client
            .delivery(Some(&bob.did), 10)
            .map_or(true, |b| b.is_empty()),
        "no delivery of another DID's messages"
    );
}

/// Two runtime nodes: A's actor sends through the mediator; B's relay mailbox picks up with
/// Pickup 3.0, journals with the verified origin, then acknowledges.
fn runtime_nodes(x: &Setup<'_>) {
    let w = x.w;
    let km_b = Arc::new(InMemoryKeyManager::new());
    let probe = client(&peer(&km_b, "http://unused.invalid", &[]), &km_b, x);
    let url = probe.endpoint().to_owned();
    let rk = vec![probe.routing_key().expect("key")];
    let b_id = peer(&km_b, &url, &rk);
    let b_client = Arc::new(client(&b_id, &km_b, x));
    b_client.request_mediation().expect("mediate-grant");
    b_client
        .add_recipients(std::slice::from_ref(&b_id.did))
        .expect("keylist-update");

    let didcomm: Arc<dyn RelayProtocol> = Arc::new(DidCommProtocol);
    let mut a = w.org(OrgSpec {
        protocols: vec![didcomm.clone()],
        net: Some(x.net.clone()),
        ..OrgSpec::default()
    });
    let mut b = w.org(OrgSpec {
        protocols: vec![didcomm.clone()],
        net: Some(x.net.clone()),
        km: Some(km_b.clone()),
        bindings: vec![PickupBinding {
            name: "identus".into(),
            source: b_client.clone(),
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

    a.node
        .send(
            &addr(&a_id.did),
            ev(
                "send",
                obj(&[
                    ("to", s(&b_id.did)),
                    ("event", s("order.placed")),
                    ("payload", obj(&[("n", Value::Number(1.0))])),
                ]),
            ),
        )
        .expect("local send");
    settle(&mut [&mut a]);
    assert_eq!(b_client.status(Some(&b_id.did)).expect("status"), 1);

    settle(&mut [&mut a, &mut b]);
    let got = inputs(&b.node, &addr(&b_id.did));
    let placed = got
        .iter()
        .find(|e| e.name == "order.placed")
        .unwrap_or_else(|| panic!("delivered through the mediator: {got:?}"));
    assert_eq!(placed.origin.as_deref(), Some(a_id.did.as_str()));
    assert!(placed.originverified);
    assert_eq!(
        b_client.status(Some(&b_id.did)).expect("status"),
        0,
        "acknowledged with messages-received after the journal append"
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Message Pickup#Pickup batch]] {level: e2e}
// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Forward Routing#One mediator]] {level: e2e}
#[test]
fn interop_with_the_identus_mediator() {
    let (containers, url) = match std::env::var("VERTEP_MEDIATOR_URL") {
        Ok(url) => (None, url),
        Err(_) if std::env::var("VERTEP_MEDIATOR_INTEROP").as_deref() == Ok("1") => {
            match start_mediator() {
                Ok((c, url)) => (Some(c), url),
                Err(e) => {
                    eprintln!("skipped: the mediator could not be started in Docker: {e}");
                    return;
                }
            }
        }
        Err(_) => {
            eprintln!(
                "skipped: set VERTEP_MEDIATOR_INTEROP=1 (Docker) or VERTEP_MEDIATOR_URL to run \
                 against {MEDIATOR_IMAGE}"
            );
            return;
        }
    };
    let med = match mediator_did(&url) {
        Ok(d) => d,
        Err(e) if containers.is_some() => panic!("{e}"),
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let w = World::new();
    let x = Setup {
        w: &w,
        net: Arc::new(Http),
        med: &med,
    };
    wire_protocol(&x);
    runtime_nodes(&x);
    drop(containers);
}

/// The same wire path against the in-process test mediator, which answers Mediator
/// Coordination 2.0 and Message Pickup 3.0 like the third-party one: runs everywhere.
// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Message Pickup#Pickup batch]] {level: integration}
#[test]
fn pickup_wire_protocol_with_the_test_mediator() {
    let w = World::new();
    let m = mediator(&w);
    let x = Setup {
        w: &w,
        net: w.net.clone(),
        med: m.did(),
    };
    wire_protocol(&x);
    runtime_nodes(&x);
}
