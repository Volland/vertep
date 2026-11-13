//! `webvh-hosting`, Third-Party Logs Interoperate: the DIF test-suite vectors publish and
//! resolve through Vertep, and logs Vertep writes resolve with `didwebvh-rs` on its own.

mod common;

use std::path::Path;

use didwebvh_rs::DIDWebVHState;
use didwebvh_rs::log_entry::LogEntryMethods as _;
use serde_json::Value;
use vertep_federation::KeyType;
use vertep_webvh::witness::CustodianWitness;
use vertep_webvh::{CreateOptions, DidLog, DirectoryPublisher, LogPublisher, Witnessing};

/// Every vector resolves except `witness-update`, which lowers its own witness threshold and
/// supplies one proof. `didwebvh-rs` rejects it on purpose (witness changes apply only after the
/// entry that makes them; see its 0.8 changelog), and so does Vertep: otherwise a stolen update
/// key could witness its own entry, and custodians could be bypassed.
const SCENARIOS: [&str; 12] = [
    "basic-create",
    "basic-update",
    "deactivate",
    "key-rotation",
    "multi-update",
    "multiple-update-keys",
    "portable",
    "portable-move",
    "pre-rotation",
    "pre-rotation-consume",
    "services",
    "witness-threshold",
];

fn vector(name: &str) -> (String, Option<String>, Value) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors")
        .join(name);
    let log = std::fs::read_to_string(dir.join("did.jsonl")).unwrap();
    let witness = std::fs::read_to_string(dir.join("did-witness.json")).ok();
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("resolutionResult.json")).unwrap())
            .unwrap();
    (log, witness, expected)
}

// @tg: verifies:: [[openspec:webvh-hosting#Third-Party Logs Interoperate#Reference test vector resolves]] {level: integration}
#[test]
fn reference_test_vector_resolves() {
    for name in SCENARIOS {
        let (text, witness, expected) = vector(name);
        let log = DidLog::from_files(&text, witness.as_deref()).unwrap();
        let did = expected["didDocument"]["id"].as_str().unwrap();
        assert_eq!(log.did(), did, "{name}");
        let version = expected["didDocumentMetadata"]["versionId"]
            .as_str()
            .unwrap();

        let v = log.verify().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(v.version_id, version, "{name}");
        assert_eq!(
            v.deactivated,
            expected["didDocumentMetadata"]["deactivated"] == true,
            "{name}"
        );

        let site = tempfile::tempdir().unwrap();
        DirectoryPublisher::new(site.path())
            .publish(&log)
            .unwrap_or_else(|e| panic!("{name}: publish: {e}"));
        let r = common::resolve(site.path(), did);
        assert_eq!(r.document.id, did, "{name}");
        assert_eq!(r.version_id.as_deref(), Some(version), "{name}");
    }
}

// @tg: verifies:: [[openspec:webvh-hosting#Third-Party Logs Interoperate#Reference test vector resolves]] {level: integration}
#[test]
fn self_lowered_witness_threshold_refused() {
    let (text, witness, _) = vector("witness-update");
    let log = DidLog::from_files(&text, witness.as_deref()).unwrap();
    match log.verify() {
        Err(vertep_webvh::WebvhError::Unwitnessed { have, need, .. }) => {
            assert_eq!((have, need), (1, 2));
        }
        other => panic!("expected the entry refused for too few witnesses, got {other:?}"),
    }
    let mut state = DIDWebVHState::default();
    assert!(
        futures::executor::block_on(state.resolve_log_owned(log.did(), &text, witness.as_deref()))
            .is_err(),
        "didwebvh-rs refuses it too"
    );
}

/// A Vertep-written log with every feature: pre-rotation, witnesses, updates, deactivation.
#[test]
fn vertep_logs_resolve_with_didwebvh_rs_alone() {
    let keys = common::keys();
    let wkeys: Vec<CustodianWitness> = (0..3)
        .map(|_| {
            CustodianWitness::new(keys.clone(), keys.generate(KeyType::Ed25519).unwrap()).unwrap()
        })
        .collect();
    let mut author = common::author(&keys, true);
    let options = CreateOptions::new("example.org")
        .path("objects:ticket-8812")
        .pre_rotation(true)
        .ttl(120)
        .witnesses(Witnessing {
            witnesses: wkeys.iter().map(CustodianWitness::did_key).collect(),
            threshold: 2,
        });
    let (_, doc) = common::document(&keys, &options, vec![common::tsp_service("https://h/tsp")]);
    let mut log = author.create(&options, &doc).unwrap();
    let witness_last = |log: &mut DidLog| {
        let vid = log.version_id().unwrap().to_owned();
        for w in &wkeys[..2] {
            log.add_witness_proof(&vid, &w.prove(&vid).unwrap())
                .unwrap();
        }
    };
    witness_last(&mut log);
    let current = log.document().cloned().unwrap();
    author.update(&mut log, current).unwrap();
    witness_last(&mut log);
    author.rotate(&mut log, None).unwrap();
    witness_last(&mut log);
    author.deactivate(&mut log).unwrap();
    // Deactivation with pre-rotation writes two entries; witness both.
    let entries = log.entries().len();
    let vid_before = log.entries()[entries - 2]["versionId"]
        .as_str()
        .unwrap()
        .to_owned();
    for w in &wkeys[..2] {
        log.add_witness_proof(&vid_before, &w.prove(&vid_before).unwrap())
            .unwrap();
    }
    witness_last(&mut log);

    let mut state = DIDWebVHState::default();
    let (entry, meta) = futures::executor::block_on(state.resolve_log_owned(
        log.did(),
        &log.jsonl(),
        log.witness_json().as_deref(),
    ))
    .unwrap();
    assert!(meta.deactivated);
    assert_eq!(entry.get_version_id(), log.version_id().unwrap());
    assert_eq!(meta.version_number as usize, log.len());
}
