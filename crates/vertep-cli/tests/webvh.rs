//! `vt webvh`: the `webvh-hosting` command-line scenarios, run against the binary.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use vertep_federation::{DidDocument, FileKeyManager, KeyManager, KeyType};
use vertep_objects::custody::{Custodian, CustodyConfig};

const VT: &str = env!("CARGO_BIN_EXE_vt");

fn vt(args: &[&str]) -> Output {
    Command::new(VT).args(args).output().expect("vt runs")
}

fn ok(args: &[&str]) -> String {
    let out = vt(args);
    assert!(
        out.status.success(),
        "vt {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn field<'a>(out: &'a str, name: &str) -> &'a str {
    out.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}: ")))
        .unwrap_or_else(|| panic!("no {name} in {out}"))
}

// @tg: verifies:: [[openspec:webvh-hosting#vt webvh Commands#Create then verify]] {level: e2e}
#[test]
fn create_then_verify() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = tmp.path().join("keys");
    let site = tmp.path().join("site");
    let k = keys.to_str().unwrap();
    let s = site.to_str().unwrap();

    let out = ok(&[
        "webvh",
        "create",
        "--domain",
        "example.org",
        "--keys",
        k,
        "--out",
        s,
    ]);
    let did = out.lines().next().unwrap();
    assert!(
        did.starts_with("did:webvh:") && did.ends_with(":example.org"),
        "{out}"
    );
    let log = site.join("example.org/.well-known/did.jsonl");
    assert!(
        out.contains(&format!("published: {}", log.display())),
        "{out}"
    );

    let verified = ok(&["webvh", "verify", log.to_str().unwrap()]);
    assert_eq!(
        verified.lines().take(2).collect::<Vec<_>>(),
        vec![format!("valid: {did}"), "versions: 1".to_owned()]
    );

    // Update, rotate and verify again: the entries get increasing versionTimes on the
    // system clock.
    ok(&[
        "webvh",
        "update",
        did,
        "--keys",
        k,
        "--out",
        s,
        "--service",
        "TSPTransport=https://example.org/tsp",
    ]);
    ok(&["webvh", "rotate", did, "--keys", k, "--out", s]);
    let verified = ok(&["webvh", "verify", log.to_str().unwrap()]);
    assert!(verified.contains("versions: 3"), "{verified}");

    // A broken log fails verification with status 1.
    let text = std::fs::read_to_string(&log).unwrap();
    let broken = tmp.path().join("broken/did.jsonl");
    std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
    std::fs::write(&broken, text.replacen("TSPTransport", "TSPTransporT", 1)).unwrap();
    let out = vt(&["webvh", "verify", broken.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("invalid:"));
}

fn custodian_key(dir: &Path) -> (String, String) {
    let km = FileKeyManager::open(dir).unwrap();
    let name = km.generate(KeyType::Ed25519).unwrap();
    let mk = km.public_key(&name).unwrap().multikey();
    (name, mk)
}

// @tg: verifies:: [[openspec:webvh-hosting#vt webvh Commands#Object with custodians]] {level: e2e}
#[test]
fn object_with_custodians() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = tmp.path().join("keys");
    let k = keys.to_str().unwrap();
    let custodians: Vec<(String, std::path::PathBuf, String, String)> = ["a", "b", "k"]
        .iter()
        .map(|n| {
            let dir = tmp.path().join(format!("custodian-{n}"));
            let (name, mk) = custodian_key(&dir);
            (format!("did:web:{n}.example"), dir, name, mk)
        })
        .collect();
    let spec = |i: usize, role: &str| format!("{role},{},{}", custodians[i].0, custodians[i].3);
    let (a, b, kk) = (spec(0, "host"), spec(1, "host"), spec(2, "keeper"));
    let out = ok(&[
        "webvh",
        "object",
        "create",
        "--domain",
        "example.org",
        "--path",
        "objects:ticket-8812",
        "--keys",
        k,
        "--custodian",
        &a,
        "--custodian",
        &b,
        "--custodian",
        &kk,
        "--quorum",
        "2",
    ]);
    let did = out.lines().next().unwrap().to_owned();
    assert!(did.ends_with(":example.org:objects:ticket-8812"), "{out}");
    assert!(out.contains("custodians: 3 (quorum 2)"), "{out}");
    assert!(out.contains("witnesses: 0 of 2 proofs"), "{out}");

    let log_path = std::path::PathBuf::from(field(&out, "log"));
    let entry: Value = serde_json::from_str(
        std::fs::read_to_string(&log_path)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let witness = &entry["parameters"]["witness"];
    assert_eq!(witness["threshold"], 2);
    let ids: Vec<&str> = witness["witnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    let expected: Vec<String> = custodians
        .iter()
        .map(|c| format!("did:key:{}", c.3))
        .collect();
    assert_eq!(ids, expected);
    let doc = DidDocument::from_json(&entry["state"]).unwrap();
    assert_eq!(
        CustodyConfig::from_document(&doc).unwrap(),
        CustodyConfig::new(
            vec![
                Custodian::host("did:web:a.example"),
                Custodian::host("did:web:b.example"),
                Custodian::keeper("did:web:k.example"),
            ],
            2
        )
        .unwrap()
    );
    assert!(
        !entry["parameters"]["nextKeyHashes"].is_null(),
        "pre-rotation is on"
    );

    // Two custodians witness, then the controller publishes.
    let site = tmp.path().join("site");
    let publish = [
        "webvh",
        "publish",
        &did,
        "--keys",
        k,
        "--out",
        site.to_str().unwrap(),
    ];
    assert_eq!(vt(&publish).status.code(), Some(1), "unwitnessed: refused");
    for c in &custodians[..2] {
        let w = ok(&[
            "webvh",
            "witness",
            log_path.to_str().unwrap(),
            "--keys",
            c.1.to_str().unwrap(),
            "--key",
            &c.2,
        ]);
        assert!(w.contains(&format!("as did:key:{}", c.3)), "{w}");
    }
    ok(&publish);
    let published = site.join("example.org/objects/ticket-8812/did.jsonl");
    assert!(ok(&["webvh", "verify", published.to_str().unwrap()]).contains("versions: 1"));

    // The controller prepares a custody change: the next entry and the signed request.
    let (_, _, _, mk2) = {
        let dir = tmp.path().join("custodian-k2");
        let (name, mk) = custodian_key(&dir);
        ((), dir, name, mk)
    };
    let k2 = format!("keeper,did:web:k2.example,{mk2}");
    let request = tmp.path().join("request.json");
    let out = ok(&[
        "webvh",
        "custody",
        "change",
        &did,
        "--keys",
        k,
        "--custodian",
        &a,
        "--custodian",
        &b,
        "--custodian",
        &k2,
        "--quorum",
        "2",
        "--request-out",
        request.to_str().unwrap(),
    ]);
    assert!(field(&out, "version").starts_with("2-"), "{out}");
    let req: Value = serde_json::from_str(&std::fs::read_to_string(&request).unwrap()).unwrap();
    assert_eq!(req["object"], did.as_str());
    assert_eq!(req["config"]["version"], 2);
    assert_eq!(req["did_log_entry"]["versionId"], field(&out, "version"));
    assert!(!req["signature"].as_str().unwrap().is_empty());
}
