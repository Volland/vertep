//! The actor system's side of the WebSocket transport, run natively: the test plays the server
//! on the system's link (the browser shell connects the same link with the browser's
//! `WebSocket`; the end-to-end tests in `bindings/js` use a real native server).

use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use serde_json::{Value as J, json};
use vertep_runtime::Shell;
use vertep_runtime::websocket::{Frame, Inbound, Message, WEBSOCKET_PROCESSOR};
use vertep_wasm::{Code, Store, System, SystemOptions};

const CLIENT_TYPE: &str = "https://client.example/charts/client";
const SERVER_ACTOR: &str = "https://server.example/charts/server/s1";

const CLIENT: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="c" datamodel="cel">
  <datamodel><data id="n" expr="0"/><data id="ot" expr="''"/></datamodel>
  <state id="c">
    <transition event="go"><send type="websocket" event="ping" target="https://server.example/charts/server/s1"/></transition>
    <transition event="pong"><assign location="ctx.n" expr="ctx.n + 1"/><assign location="ctx.ot" expr="ev.origintype"/></transition>
  </state>
</scxml>"#;

fn system() -> System {
    let opts = SystemOptions::from_json(&json!({
        "storage": "memory",
        "websocket": { "url": "ws://127.0.0.1:1/vertep", "targets": ["https://server.example/"] },
    }))
    .unwrap();
    let store = Store::open(&opts.store, None).unwrap();
    let (shell, _clock) = Shell::virtual_time(1_000);
    let mut s = System::new(&opts, store, shell, None).unwrap();
    s.command(
        "register",
        &json!({ "source": CLIENT, "type": CLIENT_TYPE }),
    )
    .unwrap();
    s
}

fn frames(rx: &mut UnboundedReceiver<String>) -> Vec<Frame> {
    let mut out = Vec::new();
    while let Ok(t) = rx.try_recv() {
        out.push(Frame::parse(&t).unwrap());
    }
    out
}

fn messages(rx: &mut UnboundedReceiver<String>) -> Vec<Message> {
    frames(rx)
        .into_iter()
        .filter_map(|f| match f {
            Frame::Message(m) => Some(m),
            _ => None,
        })
        .collect()
}

#[test]
fn options_need_a_websocket_url() {
    let bad = SystemOptions::from_json(
        &json!({ "storage": "memory", "websocket": { "url": "http://x" } }),
    );
    assert_eq!(bad.unwrap_err().code, Code::InvalidArgument);
}

// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Browser actor sends to a server actor]] {level: unit}
#[test]
fn chart_send_goes_out_as_an_envelope_until_acknowledged() {
    let mut s = system();
    let (link, opts) = s.websocket().unwrap();
    assert!(
        opts.node
            .as_deref()
            .unwrap()
            .starts_with("urn:vertep:node:")
    );
    let (tx, mut rx) = unbounded();
    link.connected(1, tx);
    let c1 = format!("{CLIENT_TYPE}/c1");
    s.command("send", &json!({ "to": c1, "event": { "type": "go" } }))
        .unwrap();
    let sent = messages(&mut rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].target, SERVER_ACTOR);
    assert_eq!(sent[0].origin.as_deref(), Some(c1.as_str()));
    assert_eq!(sent[0].event, "ping");
    assert!(link.is_pending(&sent[0].id), "owed until the ack");
    link.receive(
        &Frame::Ack {
            id: sent[0].id.clone(),
        }
        .to_text(),
    )
    .unwrap();
    assert_eq!(link.pending_len(), 0);
}

// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Server actor replies to a browser actor]] {level: unit}
#[test]
fn server_message_is_an_external_event_once() {
    let mut s = system();
    let (link, _) = s.websocket().unwrap();
    let (tx, mut rx) = unbounded();
    link.connected(1, tx);
    let c1 = format!("{CLIENT_TYPE}/c1");
    let pong = Frame::Message(Message {
        id: "urn:example:pong-1".into(),
        target: c1.clone(),
        origin: Some(SERVER_ACTOR.into()),
        event: "pong".into(),
        data: None,
        sendid: None,
        correlation: None,
    })
    .to_text();
    for _ in 0..2 {
        let Inbound::Deliver(env) = link.receive(&pong).unwrap() else {
            panic!("a message")
        };
        s.handle().send_envelope(env).unwrap();
        link.ack("urn:example:pong-1");
        s.pump();
    }
    assert_eq!(
        frames(&mut rx),
        vec![
            Frame::Ack {
                id: "urn:example:pong-1".into()
            };
            2
        ]
    );
    let snap = s.command("snapshot", &json!({ "address": c1 })).unwrap();
    assert_eq!(
        snap["context"]["n"],
        json!(1.0),
        "the duplicate is processed once"
    );
    assert_eq!(snap["context"]["ot"], json!(WEBSOCKET_PROCESSOR));
}

// @tg: verifies:: [[openspec:wasm-bindings#Server Interop Over WebSocket#Connection authentication is pluggable]] {level: unit}
#[test]
fn javascript_send_settles_on_ack_or_refusal() {
    let mut s = system();
    let (link, _) = s.websocket().unwrap();
    let (tx, mut rx) = unbounded();
    link.connected(1, tx);
    let r = s
        .command(
            "send",
            &json!({ "to": SERVER_ACTOR, "event": { "type": "ping" }, "messageId": "m1" }),
        )
        .unwrap();
    assert_eq!(r, json!({ "messageId": "m1", "pending": true }));
    assert_eq!(messages(&mut rx).len(), 1);
    link.receive(r#"{"type":"ack","id":"m1"}"#).unwrap();
    s.pump();
    let n: Vec<J> = s.take_notifications();
    assert_eq!(
        n,
        vec![json!({ "kind": "delivery", "messageId": "m1", "ok": true })]
    );

    s.command(
        "send",
        &json!({ "to": SERVER_ACTOR, "event": { "type": "ping" }, "messageId": "m2" }),
    )
    .unwrap();
    link.receive(r#"{"type":"refused","reason":"expired token"}"#)
        .unwrap();
    s.pump();
    let n = s.take_notifications();
    assert_eq!(n.len(), 1);
    assert_eq!(n[0]["ok"], json!(false));
    assert_eq!(n[0]["error"]["code"], json!("TransportRefused"));
    let later = s
        .command(
            "send",
            &json!({ "to": SERVER_ACTOR, "event": { "type": "ping" } }),
        )
        .unwrap_err();
    assert_eq!(later.code, Code::TransportRefused);
}
