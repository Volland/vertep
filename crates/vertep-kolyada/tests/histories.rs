//! Recorded histories for the Lean protocol model (m10-agent-protocols 8.5).
//!
//! Each scenario plays the quote protocol of `common` with real signers and records what one
//! participant's [`Instance`] saw and did, as JSON lines: a `setup` line (the protocol as the
//! checker reads it, the bindings, the participant's DID, its genesis, its projection, and its
//! status, knowledge and heads), then one line per `receive`, `hidden`, `expire` or own `send`,
//! with the events the instance returned and its status, knowledge and heads after it. A message
//! is recorded as its body with its hash and two facts the model takes as given: whether its
//! signature verifies (`sig`) and whether its payload matches the declared shape (`shape`).
//!
//! The lines must equal `lean/vertep-lean/VertepTests/data/kolyada/*.jsonl` (rewrite them with
//! `VT_BLESS=1`). The Lean test `VertepTests.Protocol` replays those files at build time through
//! the model in strict mode, and so does `lake exe vertep-kolyada --history`, which this test also
//! runs when the Lean build is present (`VT_LEAN_BIN`, or `lean/vertep-lean/.lake/build/bin`),
//! together with a doctored copy it must reject at the doctored line.
#![allow(missing_docs)]

mod common;

use std::path::PathBuf;

use common::{AUDITOR, BUYER, SELLER, pending};
use serde_json::{Value, json};
use vertep_kolyada::testkit::SoftSigner;
use vertep_kolyada::{Action, Body, Event, Instance, Message, Protocol, Status, Verifier, control};

fn is_empty_payload(v: &Value) -> bool {
    v.is_null() || v.as_object().is_some_and(serde_json::Map::is_empty)
}

/// A message as the model reads it.
fn abstract_message(p: &Protocol, m: &Message, v: &dyn Verifier) -> Value {
    let b = &m.body;
    let shape = match p.messages().get(&b.kind) {
        Some(t) => match &t.shape {
            Some(s) => s.validate(&b.payload).is_ok(),
            None => is_empty_payload(&b.payload),
        },
        None => true,
    };
    json!({
        "hash": m.hash(),
        "sig": m.verify(v).is_ok(),
        "protocol": b.protocol,
        "instance": b.instance,
        "sender": b.sender,
        "role": b.role,
        "seq": b.seq,
        "deps": b.deps,
        "recipients": b.recipients,
        "type": b.kind,
        "payloadEmpty": is_empty_payload(&b.payload),
        "shape": shape,
    })
}

fn status_text(s: Status) -> &'static str {
    match s {
        Status::Pending => "pending",
        Status::Open => "open",
        Status::Declined => "declined",
        Status::Completed => "completed",
    }
}

fn event_json(e: &Event) -> Value {
    match e {
        Event::Applied(h) => json!({ "event": "applied", "hash": h }),
        Event::Duplicate(h) => json!({ "event": "duplicate", "hash": h }),
        Event::Held(h) => json!({ "event": "held", "hash": h }),
        Event::Refused { hash, reason, .. } => {
            json!({ "event": "refused", "hash": hash, "reason": reason.as_str() })
        }
        Event::Stalled { hash, missing } => {
            json!({ "event": "stalled", "hash": hash, "missing": missing })
        }
        Event::Equivocation(eq) => {
            json!({ "event": "equivocation", "hash": eq.b.hash(), "with": eq.a.hash() })
        }
    }
}

/// The JSON lines of one participant's history.
struct History {
    name: &'static str,
    lines: Vec<Value>,
}

impl History {
    /// The setup line, for the participant `did` whose instance `inst` was just made from
    /// `genesis`.
    fn start(
        name: &'static str,
        inst: &Instance,
        genesis: &Message,
        did: &str,
        v: &dyn Verifier,
    ) -> Self {
        let p = inst.protocol();
        let transitions: Vec<Value> = p
            .transitions()
            .iter()
            .map(|t| {
                json!({ "from": t.from, "sender": t.sender, "receivers": t.receivers,
                        "message": t.message, "to": t.to })
            })
            .collect();
        let projection: Vec<Value> = p
            .project(inst.local_role())
            .expect("the local role is a role")
            .steps
            .iter()
            .map(|s| {
                let action = match &s.action {
                    Action::Send { message, receivers } => {
                        json!({ "send": message, "receivers": receivers })
                    }
                    Action::Receive { message, sender } => {
                        json!({ "receive": message, "sender": sender })
                    }
                    Action::Silent => json!("silent"),
                };
                json!({ "from": s.from, "to": s.to, "action": action })
            })
            .collect();
        let mut line = json!({
            "kind": "setup",
            "protocol": {
                "id": p.id().as_str(),
                "roles": p.roles(),
                "messages": p.messages().keys().collect::<Vec<_>>(),
                "initial": p.initial(),
                "finals": p.finals(),
                "transitions": transitions,
                "bufferSize": p.limits().buffer_size,
                "timeoutMs": p.limits().timeout_ms,
            },
            "bindings": inst.bindings(),
            "local": did,
            "genesis": abstract_message(p, genesis, v),
            "projection": projection,
        });
        Self::state(&mut line, inst);
        Self {
            name,
            lines: vec![line],
        }
    }

    fn state(line: &mut Value, inst: &Instance) {
        line["status"] = json!(status_text(inst.status()));
        line["knowledge"] = json!(inst.knowledge());
        line["heads"] = json!(inst.heads());
    }

    fn push(&mut self, mut line: Value, inst: &Instance, events: &[Event]) {
        line["events"] = Value::Array(events.iter().map(event_json).collect());
        Self::state(&mut line, inst);
        self.lines.push(line);
    }

    fn receive(
        &mut self,
        inst: &mut Instance,
        m: &Message,
        v: &dyn Verifier,
        now: u64,
    ) -> Vec<Event> {
        let msg = abstract_message(inst.protocol(), m, v);
        let events = inst.receive(m, v, now);
        self.push(
            json!({ "kind": "receive", "msg": msg, "now": now }),
            inst,
            &events,
        );
        events
    }

    fn hidden(&mut self, inst: &mut Instance, hash: &str, v: &dyn Verifier) -> Vec<Event> {
        let events = inst.observe_hidden(hash, v);
        self.push(json!({ "kind": "hidden", "hash": hash }), inst, &events);
        events
    }

    fn expire(&mut self, inst: &mut Instance, now: u64) -> Vec<Event> {
        let events = inst.expire(now);
        self.push(json!({ "kind": "expire", "now": now }), inst, &events);
        events
    }

    /// Records a message the participant has just sent with `inst`.
    fn sent(&mut self, inst: &Instance, m: &Message, v: &dyn Verifier) {
        let msg = abstract_message(inst.protocol(), m, v);
        let mut line = json!({ "kind": "send", "msg": msg });
        Self::state(&mut line, inst);
        self.lines.push(line);
    }

    fn text(&self) -> String {
        self.lines.iter().map(|l| format!("{l}\n")).collect()
    }
}

fn request() -> Value {
    json!({ "item": "widget", "qty": 3 })
}

fn quote_payload() -> Value {
    json!({ "price": 9.5, "currency": "EUR" })
}

fn kinds(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            Event::Applied(_) => "applied".to_owned(),
            Event::Duplicate(_) => "duplicate".to_owned(),
            Event::Held(_) => "held".to_owned(),
            Event::Refused { reason, .. } => format!("refused: {}", reason.as_str()),
            Event::Stalled { .. } => "stalled".to_owned(),
            Event::Equivocation(_) => "equivocation".to_owned(),
        })
        .collect()
}

/// A full run seen by all three participants: joins in different orders, a request that arrives
/// before a join it depends on, a quote that arrives before the hidden request it depends on, a
/// duplicate, the order and receipt hidden from the auditor, and the completion.
fn quote_run() -> Vec<History> {
    let mut s = pending();
    let v = &s.cast.verifier;
    let mut hb = History::start("quote-buyer", &s.buyer, &s.genesis, BUYER, v);
    let mut hs = History::start("quote-seller", &s.seller, &s.genesis, SELLER, v);
    let mut ha = History::start("quote-auditor", &s.auditor, &s.genesis, AUDITOR, v);

    let j_s = s.seller.join(&s.cast.seller).unwrap();
    hs.sent(&s.seller, &j_s, v);
    let j_a = s.auditor.join(&s.cast.auditor).unwrap();
    ha.sent(&s.auditor, &j_a, v);
    hb.receive(&mut s.buyer, &j_a, v, 0);
    hb.receive(&mut s.buyer, &j_s, v, 0);
    ha.receive(&mut s.auditor, &j_s, v, 0);
    assert_eq!(s.buyer.status(), Status::Open);

    let m1 = s
        .buyer
        .send(&s.cast.buyer, "quote.request", request(), &["seller"])
        .unwrap();
    hb.sent(&s.buyer, &m1, v);
    // The request depends on the auditor's join, which the seller has not seen yet.
    assert_eq!(kinds(&hs.receive(&mut s.seller, &m1, v, 5)), ["held"]);
    assert_eq!(
        kinds(&hs.receive(&mut s.seller, &j_a, v, 6)),
        ["applied", "applied"]
    );

    let m2 = s
        .seller
        .send(
            &s.cast.seller,
            "quote",
            quote_payload(),
            &["buyer", "auditor"],
        )
        .unwrap();
    hs.sent(&s.seller, &m2, v);
    // The auditor gets the quote before it learns the hash of the request hidden from it.
    assert_eq!(kinds(&ha.receive(&mut s.auditor, &m2, v, 7)), ["held"]);
    assert_eq!(
        kinds(&ha.hidden(&mut s.auditor, &m1.hash(), v)),
        ["applied"]
    );
    assert!(ha.hidden(&mut s.auditor, &m1.hash(), v).is_empty());
    hb.receive(&mut s.buyer, &m2, v, 8);
    assert_eq!(kinds(&hb.receive(&mut s.buyer, &m2, v, 9)), ["duplicate"]);

    let m3 = s
        .buyer
        .send(&s.cast.buyer, "order", json!({}), &["seller"])
        .unwrap();
    hb.sent(&s.buyer, &m3, v);
    hs.receive(&mut s.seller, &m3, v, 10);
    ha.hidden(&mut s.auditor, &m3.hash(), v);

    let m4 = s
        .seller
        .send(&s.cast.seller, "receipt", json!({}), &["buyer"])
        .unwrap();
    hs.sent(&s.seller, &m4, v);
    hb.receive(&mut s.buyer, &m4, v, 11);
    ha.hidden(&mut s.auditor, &m4.hash(), v);

    let m5 = s
        .buyer
        .send(
            &s.cast.buyer,
            control::COMPLETE,
            json!({}),
            &["seller", "auditor"],
        )
        .unwrap();
    hb.sent(&s.buyer, &m5, v);
    assert_eq!(kinds(&hs.receive(&mut s.seller, &m5, v, 12)), ["applied"]);
    assert_eq!(kinds(&ha.receive(&mut s.auditor, &m5, v, 12)), ["applied"]);
    for i in [&s.buyer, &s.seller, &s.auditor] {
        assert_eq!(i.status(), Status::Completed);
    }
    vec![hb, hs, ha]
}

/// The buyer refuses one forged or malformed quote per reason, then applies the real one.
fn refusals() -> History {
    let mut s = pending();
    let thief = SoftSigner::from_seed("did:example:thief", [9; 32]);
    s.cast.verifier.add(&thief);
    let v = &s.cast.verifier;
    let mut hb = History::start("refusals", &s.buyer, &s.genesis, BUYER, v);
    let j_s = s.seller.join(&s.cast.seller).unwrap();
    let j_a = s.auditor.join(&s.cast.auditor).unwrap();
    s.seller.receive(&j_a, v, 0);
    hb.receive(&mut s.buyer, &j_s, v, 0);
    hb.receive(&mut s.buyer, &j_a, v, 0);
    let m1 = s
        .buyer
        .send(&s.cast.buyer, "quote.request", request(), &["seller"])
        .unwrap();
    hb.sent(&s.buyer, &m1, v);
    s.seller.receive(&m1, v, 0);

    let base = Body {
        protocol: s.buyer.protocol().id().as_str().into(),
        instance: Some(s.buyer.id().into()),
        sender: SELLER.into(),
        role: "seller".into(),
        seq: 2,
        deps: vec![m1.hash()],
        recipients: vec!["auditor".into(), "buyer".into()],
        kind: "quote".into(),
        payload: quote_payload(),
    };
    let signed = |body: Body, signer: &SoftSigner| Message::sign(body, signer).unwrap();
    let mut tampered = signed(base.clone(), &s.cast.seller);
    tampered.body.payload = json!({ "price": 0.5, "currency": "EUR" });
    let cases: Vec<(Message, &str)> = vec![
        (
            signed(
                Body {
                    payload: json!({ "price": 9.5, "currency": "GBP" }),
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "payload shape",
        ),
        (tampered, "signature"),
        (
            signed(
                Body {
                    sender: "did:example:thief".into(),
                    ..base.clone()
                },
                &thief,
            ),
            "role not permitted",
        ),
        (
            signed(
                Body {
                    sender: AUDITOR.into(),
                    ..base.clone()
                },
                &s.cast.auditor,
            ),
            "role not permitted",
        ),
        (
            signed(
                Body {
                    instance: Some("sha256:another-instance".into()),
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "wrong instance",
        ),
        (
            signed(
                Body {
                    seq: 0,
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "sequence",
        ),
        (
            signed(
                Body {
                    seq: 1,
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "equivocation",
        ),
        (
            signed(
                Body {
                    recipients: vec!["buyer".into()],
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "recipients",
        ),
        (
            signed(
                Body {
                    kind: "invoice".into(),
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "unknown message",
        ),
        (
            signed(
                Body {
                    kind: "receipt".into(),
                    payload: json!({}),
                    ..base.clone()
                },
                &s.cast.seller,
            ),
            "not enabled in state",
        ),
        (
            signed(
                Body {
                    sender: BUYER.into(),
                    role: "buyer".into(),
                    seq: 3,
                    recipients: vec!["auditor".into(), "seller".into()],
                    ..base.clone()
                },
                &s.cast.buyer,
            ),
            "role not permitted",
        ),
    ];
    for (m, reason) in &cases {
        let events = hb.receive(&mut s.buyer, m, v, 1);
        assert_eq!(
            kinds(&events).last().map(String::as_str),
            Some(format!("refused: {reason}").as_str()),
            "{reason}"
        );
    }
    let m2 = s
        .seller
        .send(
            &s.cast.seller,
            "quote",
            quote_payload(),
            &["buyer", "auditor"],
        )
        .unwrap();
    assert_eq!(kinds(&hb.receive(&mut s.buyer, &m2, v, 2)), ["applied"]);
    assert_eq!(kinds(&hb.receive(&mut s.buyer, &m2, v, 3)), ["duplicate"]);
    let m3 = s
        .buyer
        .send(&s.cast.buyer, "cancel", json!({}), &["seller"])
        .unwrap();
    hb.sent(&s.buyer, &m3, v);
    hb
}

/// The auditor declines; the instance never opens, and a later join is refused.
fn declined() -> History {
    let mut s = pending();
    let v = &s.cast.verifier;
    let mut hb = History::start("declined", &s.buyer, &s.genesis, BUYER, v);
    let d = s.auditor.decline(&s.cast.auditor).unwrap();
    assert_eq!(kinds(&hb.receive(&mut s.buyer, &d, v, 0)), ["applied"]);
    let j = s.seller.join(&s.cast.seller).unwrap();
    assert_eq!(
        kinds(&hb.receive(&mut s.buyer, &j, v, 1)),
        ["refused: not open"]
    );
    assert_eq!(s.buyer.status(), Status::Declined);
    hb
}

/// The seller holds requests with a dependency that never arrives: the buffer of four evicts the
/// oldest, and the timeout of 1000 ms reports the others as stalls.
fn buffer() -> History {
    let mut s = pending();
    let v = &s.cast.verifier;
    let mut hs = History::start("buffer", &s.seller, &s.genesis, SELLER, v);
    let j_s = s.seller.join(&s.cast.seller).unwrap();
    hs.sent(&s.seller, &j_s, v);
    let j_a = s.auditor.join(&s.cast.auditor).unwrap();
    hs.receive(&mut s.seller, &j_a, v, 0);
    let missing = format!("sha256:{}", "0".repeat(64));
    for i in 0..5u64 {
        let body = Body {
            protocol: s.seller.protocol().id().as_str().into(),
            instance: Some(s.seller.id().into()),
            sender: BUYER.into(),
            role: "buyer".into(),
            seq: 10 + i,
            deps: vec![missing.clone()],
            recipients: vec!["seller".into()],
            kind: "quote.request".into(),
            payload: request(),
        };
        let m = Message::sign(body, &s.cast.buyer).unwrap();
        hs.receive(&mut s.seller, &m, v, 100 * i);
        if i == 1 {
            // A copy of a held message is held once.
            assert_eq!(kinds(&hs.receive(&mut s.seller, &m, v, 150)), ["held"]);
        }
    }
    assert_eq!(s.seller.held_len(), 4);
    assert!(hs.expire(&mut s.seller, 1099).is_empty());
    assert_eq!(kinds(&hs.expire(&mut s.seller, 1150)), ["stalled"]);
    assert_eq!(kinds(&hs.expire(&mut s.seller, 5000)).len(), 3);
    hs
}

fn all_histories() -> Vec<History> {
    let mut hs = quote_run();
    hs.push(refusals());
    hs.push(declined());
    hs.push(buffer());
    hs
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../lean/vertep-lean/VertepTests/data/kolyada")
}

fn lean_checker() -> Option<PathBuf> {
    let dir = std::env::var_os("VT_LEAN_BIN").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lean/vertep-lean/.lake/build/bin"),
        PathBuf::from,
    );
    let exe = dir.join("vertep-kolyada");
    exe.is_file().then_some(exe)
}

/// The histories of the protocol runs equal the golden files the Lean model replays, and, when
/// the Lean checker is built, it accepts them and rejects a doctored copy at the doctored line.
// @tg: verifies:: [[openspec:protocol-conformance#Lean Protocol Model#Rust history replays]] {level: integration}
#[test]
fn recorded_histories_replay_in_lean() {
    let dir = golden_dir();
    let histories = all_histories();
    if std::env::var_os("VT_BLESS").is_some() {
        std::fs::create_dir_all(&dir).expect("create the golden directory");
        for h in &histories {
            std::fs::write(dir.join(format!("{}.jsonl", h.name)), h.text())
                .expect("write a golden history");
        }
    }
    for h in &histories {
        let path = dir.join(format!("{}.jsonl", h.name));
        // compared as JSON values, so the key order of the serializer does not matter
        let golden: Vec<Value> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).expect("a golden line is JSON"))
            .collect();
        assert!(
            golden == h.lines,
            "{} differs from the recorded history; rerun with VT_BLESS=1 if the change is intended\n{}",
            path.display(),
            h.text()
        );
    }

    let Some(exe) = lean_checker() else {
        eprintln!("vertep-kolyada not built; the Lean test VertepTests.Protocol replays the files");
        return;
    };
    let files: Vec<PathBuf> = histories
        .iter()
        .map(|h| dir.join(format!("{}.jsonl", h.name)))
        .collect();
    let out = std::process::Command::new(&exe)
        .arg("--history")
        .args(&files)
        .output()
        .expect("run vertep-kolyada");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Doctor the seller's history: its view of the request now claims to come from the auditor.
    let seller = &histories[1];
    let mut lines = seller.lines.clone();
    let n = lines
        .iter()
        .position(|l| l["kind"] == "receive" && l["msg"]["type"] == "quote.request")
        .expect("the seller receives the request");
    lines[n]["msg"]["sender"] = json!(AUDITOR);
    let doctored: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let tmp = std::env::temp_dir().join(format!(
        "vertep-kolyada-doctored-{}.jsonl",
        std::process::id()
    ));
    std::fs::write(&tmp, doctored).expect("write the doctored history");
    let out = std::process::Command::new(&exe)
        .arg("--history")
        .arg(&tmp)
        .output()
        .expect("run vertep-kolyada");
    let _ = std::fs::remove_file(&tmp);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains(&format!(":{}: receive:", n + 1)),
        "rejected at line {}: {err}",
        n + 1
    );
}
