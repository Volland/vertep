//! `vt kolyada`: protocol documents, offers, histories and vectors from the command line.
#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use vertep_federation::{InMemoryKeyManager, KeyManager, KeyType};

const VT: &str = env!("CARGO_BIN_EXE_vt");

fn vt(args: &[&str]) -> Output {
    Command::new(VT).args(args).output().expect("vt runs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn vectors() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../vertep-kolyada/vectors")
}

fn vector(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(vectors().join(name)).unwrap()).unwrap()
}

/// The quote protocol of the published vectors, and its identity.
fn quote() -> (Value, String) {
    let case = vector("identity.json")["cases"][0].clone();
    (
        case["document"].clone(),
        case["id"].as_str().unwrap().to_owned(),
    )
}

fn write(dir: &Path, name: &str, v: &Value) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, serde_json::to_string_pretty(v).unwrap()).unwrap();
    p
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Document#Valid document accepted]] {level: e2e}
#[test]
fn validate_accepts_a_document() {
    let dir = tempfile::tempdir().unwrap();
    let (doc, id) = quote();
    let o = vt(&["kolyada", "validate", s(&write(dir.path(), "q.json", &doc))]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with(&format!("valid {id}\n")), "{out}");
    assert!(out.contains("roles: buyer, seller, auditor"), "{out}");
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Document#Unknown term refused]] {level: e2e}
#[test]
fn validate_refuses_an_unknown_term() {
    let dir = tempfile::tempdir().unwrap();
    let (mut doc, _) = quote();
    doc["deadline"] = json!("tomorrow");
    let o = vt(&["kolyada", "validate", s(&write(dir.path(), "q.json", &doc))]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("deadline"), "{}", stderr(&o));
    let missing = vt(&["kolyada", "validate", "/nonexistent/q.json"]);
    assert_eq!(missing.status.code(), Some(2));
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Identity#Spellings share an identity]] {level: e2e}
#[test]
fn hash_prints_the_identity_of_any_spelling() {
    let dir = tempfile::tempdir().unwrap();
    let (doc, id) = quote();
    let mut reordered = doc.clone();
    reordered["roles"].as_array_mut().unwrap().reverse();
    reordered["transitions"].as_array_mut().unwrap().reverse();
    for (name, d) in [("a.json", doc), ("b.json", reordered)] {
        let o = vt(&["kolyada", "hash", s(&write(dir.path(), name, &d))]);
        assert!(o.status.success(), "{}", stderr(&o));
        assert_eq!(stdout(&o), format!("{id}\n"));
    }
}

// @tg: verifies:: [[openspec:agent-protocols#Role Projection#Projection of a request and reply]] {level: e2e}
#[test]
fn project_prints_steps_and_the_interface_view() {
    let dir = tempfile::tempdir().unwrap();
    let (doc, _) = quote();
    let path = write(dir.path(), "q.json", &doc);
    let o = vt(&["kolyada", "project", s(&path), "--role", "seller"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("Idle -> Quoting: receive quote.request from buyer"),
        "{out}"
    );
    assert!(
        out.contains("Quoting -> Decided: send quote to auditor, buyer"),
        "{out}"
    );
    assert!(
        out.contains("accepts in Idle: quote.request from buyer"),
        "{out}"
    );
    // The JSON form is the form of the published projection vectors.
    let o = vt(&["kolyada", "project", s(&path), "--json"]);
    let mut v: Value = serde_json::from_slice(&o.stdout).unwrap();
    for p in v["projections"].as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("accepts");
    }
    assert_eq!(v, vector("projection.json"));
    let unknown = vt(&["kolyada", "project", s(&path), "--role", "courier"]);
    assert_eq!(unknown.status.code(), Some(1));
}

fn offer(dir: &Path, expires: Option<&str>) -> (PathBuf, Value) {
    let (doc, _) = quote();
    let path = write(dir, "q.json", &doc);
    let keys = dir.join("keys");
    let mut args = vec![
        "kolyada",
        "offer",
        s(&path),
        "--role",
        "seller",
        "--keys",
        s(&keys),
    ];
    if let Some(e) = expires {
        args.extend(["--expires-ms", e]);
    }
    let o = vt(&args);
    assert!(o.status.success(), "{}", stderr(&o));
    (path, serde_json::from_slice(&o.stdout).unwrap())
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Offers#Offer verified]] {level: e2e}
#[test]
fn offer_is_signed_and_verifies() {
    let dir = tempfile::tempdir().unwrap();
    let (doc, out) = offer(dir.path(), None);
    let did = out["offer"]["body"]["did"].as_str().unwrap().to_owned();
    assert!(did.starts_with("did:key:"));
    assert_eq!(out["service"]["type"], "VertepProtocol");
    assert_eq!(
        out["service"]["serviceEndpoint"][0]["roles"],
        json!(["seller"])
    );
    let file = write(dir.path(), "offer.json", &out["offer"]);
    let o = vt(&["kolyada", "check", s(&doc), s(&file)]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains(&format!("valid offer: {did} plays seller")));
    // A changed role no longer verifies.
    let mut forged = out["offer"].clone();
    forged["body"]["role"] = json!("buyer");
    let file = write(dir.path(), "forged.json", &forged);
    assert_eq!(
        vt(&["kolyada", "check", s(&doc), s(&file)]).status.code(),
        Some(1)
    );
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Offers#Expired offer refused]] {level: e2e}
#[test]
fn expired_offer_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (doc, out) = offer(dir.path(), Some("1000"));
    let file = write(dir.path(), "offer.json", &out["offer"]);
    let ok = vt(&["kolyada", "check", s(&doc), s(&file), "--now-ms", "999"]);
    assert!(ok.status.success(), "{}", stderr(&ok));
    let late = vt(&["kolyada", "check", s(&doc), s(&file), "--now-ms", "1000"]);
    assert_eq!(late.status.code(), Some(1));
    assert!(stderr(&late).contains("expired"), "{}", stderr(&late));
}

fn public_key(seed: &str) -> String {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(seed).unwrap().try_into().unwrap();
    let km = InMemoryKeyManager::new();
    let name = km.import_seed(KeyType::Ed25519, bytes).unwrap();
    URL_SAFE_NO_PAD.encode(km.public_key(&name).unwrap().bytes)
}

// @tg: verifies:: [[openspec:protocol-conformance#Conformance Check#Message out of state refused]] {level: e2e}
#[test]
fn check_replays_a_history_and_names_the_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let v = vector("verdicts.json");
    let doc = write(dir.path(), "q.json", &v["protocol"]);
    let setup: Vec<Value> = v["setup"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["body"]["type"] != "kolyada:genesis")
        .cloned()
        .collect();
    let step = |name: &str| {
        v["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == name)
            .unwrap()["message"]
            .clone()
    };
    let mut keys = Vec::new();
    for role in ["buyer", "seller", "auditor"] {
        keys.push("--key".to_owned());
        keys.push(format!(
            "{}={}",
            v["participants"][role].as_str().unwrap(),
            public_key(v["seeds"][role].as_str().unwrap())
        ));
    }
    let run = |history: Vec<Value>| {
        let file = write(dir.path(), "h.json", &Value::Array(history));
        let mut args: Vec<String> = ["kolyada", "check", s(&doc), s(&file), "--as"]
            .iter()
            .map(|a| (*a).to_owned())
            .collect();
        args.push(v["participants"]["seller"].as_str().unwrap().to_owned());
        args.extend(keys.clone());
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        vt(&refs)
    };
    let mut good = vec![v["genesis"].clone()];
    good.extend(setup.clone());
    good.push(step("valid request"));
    let o = run(good);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    assert!(
        stdout(&o).contains("quote.request from buyer: accepted"),
        "{}",
        stdout(&o)
    );
    assert!(stdout(&o).contains("as seller"), "{}", stdout(&o));

    let mut bad = vec![v["genesis"].clone()];
    bad.extend(setup);
    bad.push(step("message not enabled in the state"));
    let o = run(bad);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("order from buyer: refused: not enabled in state"),
        "{}",
        stderr(&o)
    );
}

// @tg: verifies:: [[openspec:protocol-conformance#Test Vectors#Vectors pass in Rust]] {level: e2e}
#[test]
fn vectors_pass_and_a_doctored_vector_fails() {
    let o = vt(&["kolyada", "vectors", s(&vectors())]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    assert_eq!(
        stdout(&o).lines().filter(|l| l.starts_with("ok")).count(),
        11
    );

    let dir = tempfile::tempdir().unwrap();
    for f in [
        "identity.json",
        "projection.json",
        "envelope.json",
        "verdicts.json",
    ] {
        std::fs::copy(vectors().join(f), dir.path().join(f)).unwrap();
    }
    let mut env = vector("envelope.json");
    env["hash"] = json!("0".repeat(64));
    write(dir.path(), "envelope.json", &env);
    let o = vt(&["kolyada", "vectors", s(dir.path())]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("FAIL envelope"), "{}", stderr(&o));
}
