//! The WebSocket Event I/O Processor between two native nodes: a server node with the
//! WebSocket endpoint and a client node whose link the test drives with a tokio-tungstenite
//! client (the browser shell drives the same link with the browser's `WebSocket`).
//!
//! Run with `cargo test -p vertep-runtime --features websocket --test websocket`.
#![cfg(feature = "websocket")]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use common::{ecma, ecma_scxml, ev};
use futures::channel::mpsc::unbounded;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use vertep_core::datamodel::Value;
use vertep_runtime::websocket::server::{ServerOptions, WebSocketServer};
use vertep_runtime::websocket::{
    ClientResolver, Frame, Inbound, Link, REFUSED_REASON, WEBSOCKET_PROCESSOR, WebSocketTransport,
};
use vertep_runtime::{Address, Node, NodeConfig, NodeHandle, Shell};
use vertep_store::{JournalStore, MemoryStore};

const SERVER_TYPE: &str = "https://server.example/charts/server";
const CLIENT_TYPE: &str = "https://client.example/charts/client";

const SERVER: &str = r#"<datamodel><data id="n" expr="0"/><data id="ot" expr="''"/><data id="from" expr="''"/></datamodel>
<state id="s"><transition event="ping">
  <assign location="n" expr="n + 1"/>
  <assign location="ot" expr="_event.origintype"/>
  <assign location="from" expr="_event.origin"/>
  <send event="pong" targetexpr="_event.origin"><content expr="({n: n})"/></send>
</transition></state>"#;

const CLIENT: &str = r#"<datamodel><data id="n" expr="0"/><data id="ot" expr="''"/></datamodel>
<state id="c">
  <transition event="go"><send type="websocket" event="ping" target="https://server.example/charts/server/s1"/></transition>
  <transition event="pong"><assign location="n" expr="n + 1"/><assign location="ot" expr="_event.origintype"/></transition>
</state>"#;

fn config() -> NodeConfig {
    NodeConfig {
        failure_backoff_ms: 20,
        ..NodeConfig::default()
    }
}

fn node(store: Arc<MemoryStore>) -> Node {
    Node::builder(store, Shell::tokio())
        .datamodels(ecma())
        .transport(Arc::new(WebSocketTransport))
        .config(config())
        .build()
}

fn head(store: &MemoryStore, a: &Address) -> u64 {
    store.head(a.node()).map(|p| p.get()).unwrap_or(0)
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// A global variable of an ECMAScript datamodel snapshot (`globals: [[name, value, flags]]`).
fn field(dm: &Value, k: &str) -> Option<Value> {
    let Value::Object(o) = dm else { return None };
    let (_, Value::Array(globals)) = o.iter().find(|(n, _)| n == "globals")? else {
        return None;
    };
    globals.iter().find_map(|g| match g {
        Value::Array(e) if e.first() == Some(&Value::String(k.into())) => e.get(1).cloned(),
        _ => None,
    })
}

/// A client connection loop over `link`: connect, `hello`, then frames until the connection
/// closes, and reconnect, until `stop`.
fn drive_client(
    url: String,
    link: Arc<Link>,
    handle: NodeHandle,
    credential: Option<String>,
    stop: Arc<AtomicBool>,
    refused: Arc<AtomicU64>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut generation = 0;
        while !stop.load(Ordering::SeqCst) {
            let Ok((ws, _)) = tokio_tungstenite::connect_async(url.as_str()).await else {
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            };
            let (mut sink, mut source) = ws.split();
            let hello = Frame::Hello {
                node: "urn:example:client-node".into(),
                credential: credential.clone(),
                hosts: vec![],
            };
            if sink
                .send(WsMessage::Text(hello.to_text().into()))
                .await
                .is_err()
            {
                continue;
            }
            generation += 1;
            let (tx, mut rx) = unbounded::<String>();
            let mut welcomed = false;
            loop {
                tokio::select! {
                    out = rx.next() => {
                        let Some(t) = out else { break };
                        if sink.send(WsMessage::Text(t.into())).await.is_err() { break; }
                    }
                    m = source.next() => match m {
                        Some(Ok(WsMessage::Text(t))) => match link.receive(t.as_str()) {
                            Ok(Inbound::Handshake(Frame::Welcome { .. })) if !welcomed => {
                                welcomed = true;
                                link.connected(generation, tx.clone());
                            }
                            Ok(Inbound::Deliver(env)) => {
                                let id = env.message_id.clone();
                                if handle.send_envelope(env).is_ok() {
                                    link.ack(&id);
                                }
                            }
                            Ok(Inbound::Refused { .. }) => {
                                refused.fetch_add(1, Ordering::SeqCst);
                            }
                            _ => {}
                        },
                        Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
                        Some(Ok(_)) => {}
                    },
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        if stop.load(Ordering::SeqCst) { break; }
                    }
                }
            }
            link.disconnected(generation);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
}

struct World {
    server_store: Arc<MemoryStore>,
    client_store: Arc<MemoryStore>,
    server: WebSocketServer,
    link: Arc<Link>,
    client: NodeHandle,
    stop: Arc<AtomicBool>,
    refused: Arc<AtomicU64>,
    runners: Vec<tokio::task::JoinHandle<Node>>,
    handles: Vec<NodeHandle>,
}

async fn world(credential: Option<&str>) -> World {
    let server_store = Arc::new(MemoryStore::new());
    let mut server_node = node(server_store.clone());
    server_node
        .register_type(SERVER_TYPE, ecma_scxml("s", SERVER))
        .unwrap();
    let server = WebSocketServer::new(
        server_node.handle(),
        ServerOptions::new("urn:example:server-node").credentials(|c| match c {
            Some("secret") => Ok(()),
            _ => Err("unknown credential".into()),
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/vertep", listener.local_addr().unwrap());
    tokio::spawn({
        let s = server.clone();
        async move { s.serve(listener).await }
    });

    let client_store = Arc::new(MemoryStore::new());
    let mut client_node = node(client_store.clone());
    client_node
        .register_type(CLIENT_TYPE, ecma_scxml("c", CLIENT))
        .unwrap();
    let link = Arc::new(Link::new());
    let client = client_node.handle();
    client.register_resolver(
        "https",
        Arc::new(ClientResolver::new(
            link.clone(),
            Some(client.clone()),
            vec!["https://server.example/".into()],
        )),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let refused = Arc::new(AtomicU64::new(0));
    drive_client(
        url,
        link.clone(),
        client.clone(),
        credential.map(str::to_owned),
        stop.clone(),
        refused.clone(),
    );
    let handles = vec![server_node.handle(), client_node.handle()];
    let runners = vec![
        tokio::task::spawn_local(async move {
            server_node.run().await;
            server_node
        }),
        tokio::task::spawn_local(async move {
            client_node.run().await;
            client_node
        }),
    ];
    World {
        server_store,
        client_store,
        server,
        link,
        client,
        stop,
        refused,
        runners,
        handles,
    }
}

impl World {
    async fn finish(self) -> (Node, Node) {
        self.stop.store(true, Ordering::SeqCst);
        for h in &self.handles {
            h.shutdown();
        }
        let mut nodes = Vec::new();
        for r in self.runners {
            nodes.push(r.await.unwrap());
        }
        let client = nodes.pop().unwrap();
        let server = nodes.pop().unwrap();
        (server, client)
    }
}

fn message_ids(store: &MemoryStore, a: &Address) -> Vec<String> {
    store
        .read(a.node(), vertep_store::Position::new(1))
        .unwrap()
        .iter()
        .filter_map(|r| vertep_runtime::EntryData::from_record(a, r).ok())
        .map(|e| e.message_id)
        .collect()
}

// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Browser actor sends to a server actor]] {level: integration}
// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Server actor replies to a browser actor]] {level: integration}
// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Reconnect redelivers the outbox]] {level: integration}
#[tokio::test]
async fn exchange_across_forced_disconnects() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let w = world(Some("secret")).await;
            let c1 = Address::virtual_actor(CLIENT_TYPE, "c1").unwrap();
            let s1 = Address::virtual_actor(SERVER_TYPE, "s1").unwrap();

            // a client actor sends to a server actor, which replies by `_event.origin`
            w.client.send(&c1, ev("go")).unwrap();
            until("the first ping and pong", || {
                head(&w.server_store, &s1) == 1 && head(&w.client_store, &c1) == 2
            })
            .await;

            // the server journals the ping but the connection drops before its ack: the
            // client's outbox sends it again after reconnecting, and it is processed once
            w.server.set_hold_acks(true);
            w.client.send(&c1, ev("go")).unwrap();
            until("the held ping", || head(&w.server_store, &s1) == 2).await;
            assert!(w.link.pending_len() > 0, "the ping is unacknowledged");
            w.server.set_hold_acks(false);
            w.server.disconnect_all();
            until("the redelivered ping to be acknowledged", || {
                w.link.pending_len() == 0 && head(&w.client_store, &c1) == 4
            })
            .await;

            // every server message is sent twice: the client processes each once
            w.server.set_duplicate(true);
            w.client.send(&c1, ev("go")).unwrap();
            until("the third pong", || head(&w.client_store, &c1) == 6).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            w.server.set_duplicate(false);

            let (server_store, client_store) = (w.server_store.clone(), w.client_store.clone());
            let refused = w.refused.load(Ordering::SeqCst);
            let (mut server, mut client) = w.finish().await;
            assert_eq!(refused, 0);
            assert_eq!(
                head(&server_store, &s1),
                3,
                "three pings, each processed once"
            );
            assert_eq!(head(&client_store, &c1), 6, "three goes and three pongs");
            let ids = message_ids(&server_store, &s1);
            let mut unique = ids.clone();
            unique.dedup();
            assert_eq!(ids, unique);

            let s = server.state(&s1).unwrap();
            assert_eq!(field(&s.datamodel, "n"), Some(Value::Number(3.0)));
            assert_eq!(
                field(&s.datamodel, "ot"),
                Some(Value::String(WEBSOCKET_PROCESSOR.into()))
            );
            assert_eq!(
                field(&s.datamodel, "from"),
                Some(Value::String(c1.as_str().into()))
            );
            let c = client.state(&c1).unwrap();
            assert_eq!(field(&c.datamodel, "n"), Some(Value::Number(3.0)));
            assert_eq!(
                field(&c.datamodel, "ot"),
                Some(Value::String(WEBSOCKET_PROCESSOR.into()))
            );
        })
        .await;
}

// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Connection authentication is pluggable]] {level: integration}
#[tokio::test]
async fn refused_credential_dead_letters_sends() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let w = world(Some("wrong")).await;
            let c1 = Address::virtual_actor(CLIENT_TYPE, "c1").unwrap();
            let s1 = Address::virtual_actor(SERVER_TYPE, "s1").unwrap();
            until("a refusal", || w.refused.load(Ordering::SeqCst) > 0).await;
            assert!(w.link.refusal().is_some());
            w.client.send(&c1, ev("go")).unwrap();
            until("the go", || head(&w.client_store, &c1) == 1).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            let server_store = w.server_store.clone();
            let (_server, client) = w.finish().await;
            assert_eq!(head(&server_store, &s1), 0, "nothing reached the server");
            let dead = client.dead_letters(&s1).unwrap();
            assert_eq!(dead.len(), 1, "{dead:?}");
            assert_eq!(dead[0].reason, REFUSED_REASON);
            assert_eq!(dead[0].event, "ping");
        })
        .await;
}
