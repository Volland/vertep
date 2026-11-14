//! The published test vectors: JSON files other implementations can check themselves against.
//!
//! `cargo test -p vertep-kolyada --test vectors` compares the files in `vectors/` with what this
//! implementation produces. `KOLYADA_BLESS=1` rewrites them.
#![allow(missing_docs)]

mod common;

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use common::{AUDITOR, BUYER, SELLER, bindings, cast, parse, quote, quote_doc};
use serde_json::{Value, json};
use vertep_kolyada::projection::{Action, Projection};
use vertep_kolyada::{Body, Instance, Message, Verdict};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vectors")
}

// @tg: implements:: [[openspec:protocol-conformance#Test Vectors]]
fn check_file(name: &str, value: &Value) {
    let path = dir().join(name);
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    if std::env::var_os("KOLYADA_BLESS").is_some() {
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&path, text).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{name} is missing; run with KOLYADA_BLESS=1"));
    assert_eq!(
        committed, text,
        "{name} differs from this implementation; run with KOLYADA_BLESS=1 if that is intended"
    );
}

fn projection_json(p: &Projection) -> Value {
    let steps: Vec<Value> = p
        .steps
        .iter()
        .map(|s| {
            let action = match &s.action {
                Action::Send { message, receivers } => json!({ "send": message, "to": receivers }),
                Action::Receive { message, sender } => {
                    json!({ "receive": message, "from": sender })
                }
                Action::Silent => json!("silent"),
            };
            json!({ "from": s.from, "to": s.to, "action": action })
        })
        .collect();
    json!({ "role": p.role, "initial": p.initial, "final": p.finals, "steps": steps })
}

// @tg: verifies:: [[openspec:protocol-conformance#Test Vectors#Vectors pass in Rust]] {level: integration}
#[test]
fn identity_vectors() {
    let mut other = quote_doc();
    other["transitions"][4]["to"] = json!("Cancelled");
    let mut reordered = quote_doc();
    reordered["roles"].as_array_mut().unwrap().reverse();
    reordered["transitions"].as_array_mut().unwrap().reverse();
    let cases: Vec<Value> = [
        ("quote", quote_doc()),
        ("quote, members reordered", reordered),
        ("quote, one target changed", other),
    ]
    .into_iter()
    .map(|(name, doc)| json!({ "name": name, "id": parse(&doc).id().as_str(), "document": doc }))
    .collect();
    check_file(
        "identity.json",
        &json!({ "about": "Protocol identities: the SHA-256 of the RDFC-1.0 canonical N-Quads of the expanded graph.", "cases": cases }),
    );
}

// @tg: verifies:: [[openspec:protocol-conformance#Test Vectors#Vectors pass in Rust]] {level: integration}
#[test]
fn projection_vectors() {
    let p = quote();
    let projections: Vec<Value> = p
        .roles()
        .iter()
        .map(|r| projection_json(&p.project(r).unwrap()))
        .collect();
    check_file(
        "projection.json",
        &json!({ "protocol": p.id().as_str(), "projections": projections }),
    );
}

fn seed(n: u8) -> String {
    URL_SAFE_NO_PAD.encode([n; 32])
}

// @tg: verifies:: [[openspec:protocol-conformance#Test Vectors#Vectors pass in Rust]] {level: integration}
#[test]
fn envelope_vectors() {
    let c = cast();
    let p = quote();
    let body = Body {
        protocol: p.id().as_str().into(),
        instance: Some("0".repeat(64)),
        sender: BUYER.into(),
        role: "buyer".into(),
        seq: 2,
        deps: vec!["1".repeat(64), "0".repeat(64)],
        recipients: vec!["seller".into()],
        kind: "quote.request".into(),
        payload: json!({ "item": "widget", "qty": 3, "note": "é" }),
    };
    let m = Message::sign(body, &c.buyer).unwrap();
    let signing = String::from_utf8(m.body.signing_bytes()).unwrap();
    check_file(
        "envelope.json",
        &json!({
            "about": "Ed25519 over the JCS (RFC 8785) bytes of the body; the hash is SHA-256 over the JCS bytes of the whole message.",
            "did": BUYER,
            "publicKey": URL_SAFE_NO_PAD.encode(c.buyer.public_key()),
            "seed": seed(1),
            "signingBytes": signing,
            "message": m,
            "hash": m.hash(),
        }),
    );
}

// @tg: verifies:: [[openspec:protocol-conformance#Test Vectors#Vectors pass in Rust]] {level: integration}
#[test]
fn verdict_vectors() {
    // The seller's view of an open instance, then four messages with the verdicts it gives.
    let mut s = common::open();
    let good = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    let mut tampered = good.clone();
    tampered.body.payload = json!({ "item": "a", "qty": 9 });
    let forged_body = Body {
        kind: "order".into(),
        payload: json!({}),
        seq: 20,
        ..good.body.clone()
    };
    let forged = Message::sign(forged_body, &s.cast.buyer).unwrap();
    let bad_shape = Message::sign(
        Body {
            payload: json!({ "item": "a" }),
            seq: 21,
            ..good.body.clone()
        },
        &s.cast.buyer,
    )
    .unwrap();
    let setup: Vec<Value> = s
        .buyer
        .history_for("seller")
        .into_iter()
        .filter_map(|e| match e {
            vertep_kolyada::HistoryEntry::Full(m) if m.body.kind.starts_with("kolyada:") => {
                Some(json!(m))
            }
            _ => None,
        })
        .collect();
    let seller =
        Instance::from_genesis(quote(), &s.genesis, SELLER, &s.cast.verifier, None).unwrap();
    let mut view = seller;
    for m in &setup {
        let m: Message = serde_json::from_value(m.clone()).unwrap();
        if m.body.kind != "kolyada:genesis" {
            view.receive(&m, &s.cast.verifier, 0);
        }
    }
    let verdict = |m: &Message| match view.check(m, &s.cast.verifier) {
        Verdict::Accepted => json!("accepted"),
        Verdict::Duplicate => json!("duplicate"),
        Verdict::Refused(r) => json!({ "refused": r.as_str() }),
    };
    let steps = vec![
        json!({ "name": "valid request", "message": good, "verdict": verdict(&good) }),
        json!({ "name": "body changed after signing", "message": tampered, "verdict": verdict(&tampered) }),
        json!({ "name": "payload misses a required field", "message": bad_shape, "verdict": verdict(&bad_shape) }),
        json!({ "name": "message not enabled in the state", "message": forged, "verdict": verdict(&forged) }),
    ];
    check_file(
        "verdicts.json",
        &json!({
            "about": "The verdicts of the conformance check for the seller of an open instance.",
            "protocol": quote_doc(),
            "bindings": bindings(),
            "participants": { "buyer": BUYER, "seller": SELLER, "auditor": AUDITOR },
            "seeds": { "buyer": seed(1), "seller": seed(2), "auditor": seed(3) },
            "genesis": s.genesis,
            "setup": setup,
            "receiver": "seller",
            "steps": steps,
        }),
    );
}

// @tg: verifies:: [[openspec:protocol-conformance#Test Vectors#Vectors consumable elsewhere]] {level: e2e}
#[test]
fn vectors_check_in_node() {
    let script = dir().join("check.mjs");
    let Ok(out) = std::process::Command::new("node").arg(&script).output() else {
        eprintln!("node is not installed; skipping the language-neutral check");
        return;
    };
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
