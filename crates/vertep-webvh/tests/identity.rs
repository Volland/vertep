//! `webvh-identity`: creating, updating, rotating and deactivating `did:webvh` DIDs with
//! key-manager keys, and verifying before publishing.

mod common;

use std::sync::Arc;

use serde_json::Value;
use vertep_federation::{DidDocument, InMemoryKeyManager, KeyManager, KeyType};
use vertep_objects::custody::{Custodian, CustodyConfig};
use vertep_webvh::{
    CreateOptions, DirectoryPublisher, KmSigner, LogPublisher, WebvhError, document,
};

#[test]
fn km_signer_signs_through_the_key_manager() {
    let keys = common::keys();
    let name = keys.generate(KeyType::Ed25519).unwrap();
    let signer = KmSigner::new(keys.clone(), &name).unwrap();
    let mk = signer.multikey();
    assert_eq!(
        didwebvh_rs::prelude::Signer::verification_method(&signer),
        format!("did:key:{mk}#{mk}")
    );
    let sig =
        futures::executor::block_on(didwebvh_rs::prelude::Signer::sign(&signer, b"entry")).unwrap();
    assert!(signer.public_key().verify(b"entry", &sig));
    let x = keys.generate(KeyType::X25519).unwrap();
    assert!(KmSigner::new(keys, x).is_err(), "only Ed25519 keys sign");
}

// @tg: verifies:: [[openspec:webvh-identity#Create With Vertep Keys#Created DID resolves]] {level: integration}
#[test]
fn created_did_resolves() {
    let keys = common::keys();
    let mut author = common::author(&keys, false);
    let options = CreateOptions::new("example.org").path("objects:ticket-8812");
    let (_, doc) = common::document(&keys, &options, vec![]);
    let log = author.create(&options, &doc).unwrap();
    assert!(log.did().starts_with("did:webvh:"));
    assert!(log.did().ends_with(":example.org:objects:ticket-8812"));

    let site = tempfile::tempdir().unwrap();
    DirectoryPublisher::new(site.path()).publish(&log).unwrap();
    let r = common::resolve(site.path(), log.did());
    assert_eq!(r.document.id, log.did());
    assert_eq!(r.version_id.as_deref(), log.version_id());
}

// @tg: verifies:: [[openspec:webvh-identity#Create With Vertep Keys#Keys stay in the key manager]] {level: integration}
#[test]
fn keys_stay_in_the_key_manager() {
    let km = Arc::new(InMemoryKeyManager::new());
    let seed = [7u8; 32];
    let update = km.import_seed(KeyType::Ed25519, seed).unwrap();
    let keys: Arc<dyn KeyManager> = km;
    let mut author = vertep_webvh::Author::new(
        keys.clone(),
        vertep_webvh::UpdateKeys {
            current: update.clone(),
            next: None,
        },
    )
    .with_clock(common::clock());
    let options = CreateOptions::new("example.org");
    let (_, doc) = common::document(&keys, &options, vec![]);
    let log = author.create(&options, &doc).unwrap();
    let site = tempfile::tempdir().unwrap();
    let published = DirectoryPublisher::new(site.path()).publish(&log).unwrap();

    use base64::Engine as _;
    let encodings = [
        seed.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        bs58::encode(seed).into_string(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(seed),
        base64::engine::general_purpose::STANDARD.encode(seed),
    ];
    let mut texts: Vec<String> = published
        .files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
    texts.push(log.jsonl());
    texts.push(format!("{author:?}"));
    for t in &texts {
        for e in &encodings {
            assert!(!t.contains(e.as_str()), "private key material leaked");
        }
    }
    let update_keys = log.entries()[0]
        .pointer("/parameters/updateKeys")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(
        update_keys,
        &vec![Value::String(keys.public_key(&update).unwrap().multikey())]
    );
}

// @tg: verifies:: [[openspec:webvh-identity#Document From An Identity#Services carried into the document]] {level: integration}
#[test]
fn services_carried_into_the_document() {
    let keys = common::keys();
    let mut author = common::author(&keys, false);
    let options = CreateOptions::new("example.org").path("objects:ticket-8812");
    let identity = document::provision(
        keys.as_ref(),
        &options,
        vec![common::tsp_service("https://hosts.example/tsp")],
    )
    .unwrap();
    let custody = CustodyConfig::new(
        vec![
            Custodian::host("did:web:a.example"),
            Custodian::host("did:web:b.example"),
            Custodian::keeper("did:web:c.example"),
        ],
        2,
    )
    .unwrap();
    let controller = keys.generate(KeyType::Ed25519).unwrap();
    let doc = document::object_document(
        &identity,
        keys.as_ref(),
        &custody,
        &keys.public_key(&controller).unwrap(),
    )
    .unwrap();
    let log = author.create(&options, &doc).unwrap();
    let site = tempfile::tempdir().unwrap();
    DirectoryPublisher::new(site.path()).publish(&log).unwrap();

    let resolved = common::resolve(site.path(), log.did()).document;
    assert!(
        resolved.service.iter().any(
            |s| s.has_type("TSPTransport") && s.endpoints[0].uri == "https://hosts.example/tsp"
        )
    );
    assert_eq!(resolved.authentication.len(), 1);
    assert_eq!(resolved.key_agreement.len(), 1);
    let declared = CustodyConfig::from_document(&resolved).unwrap();
    assert_eq!(declared, custody);
}

fn with_service(doc: &Value, url: &str) -> Value {
    let mut d = DidDocument::from_json(doc).unwrap();
    d.service.retain(|s| !s.has_type("TSPTransport"));
    let mut s = common::tsp_service(url);
    s.id = format!("{}#tsp", d.id);
    d.service.push(s);
    d.to_json()
}

// @tg: verifies:: [[openspec:webvh-identity#Update And Rotate#Update keeps the DID]] {level: integration}
#[test]
fn update_keeps_the_did() {
    let keys = common::keys();
    let mut author = common::author(&keys, false);
    let options = CreateOptions::new("example.org");
    let (_, doc) = common::document(&keys, &options, vec![]);
    let mut log = author.create(&options, &doc).unwrap();
    let did = log.did().to_owned();
    let new = with_service(log.document().unwrap(), "https://new.example/tsp");
    author.update(&mut log, new).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log.did(), did);

    let site = tempfile::tempdir().unwrap();
    DirectoryPublisher::new(site.path()).publish(&log).unwrap();
    let r = common::resolve(site.path(), &did);
    assert_eq!(r.document.id, did);
    assert!(
        r.document
            .service
            .iter()
            .any(|s| s.endpoints[0].uri == "https://new.example/tsp")
    );
}

// @tg: verifies:: [[openspec:webvh-identity#Update And Rotate#Rotation with pre-rotation]] {level: integration}
#[test]
fn rotation_with_pre_rotation() {
    let keys = common::keys();
    let mut author = common::author(&keys, true);
    let options = CreateOptions::new("example.org").pre_rotation(true);
    let (_, doc) = common::document(&keys, &options, vec![]);
    let mut log = author.create(&options, &doc).unwrap();
    let old = author.update_keys().current.clone();
    let announced = author.update_keys().next.clone().unwrap();

    author.rotate(&mut log, None).unwrap();
    assert_eq!(author.update_keys().current, announced);
    assert_eq!(log.len(), 2);
    log.verify().unwrap();

    // The old update key can no longer sign an entry.
    let mut stale = vertep_webvh::Author::new(
        keys.clone(),
        vertep_webvh::UpdateKeys {
            current: old.clone(),
            next: Some(old),
        },
    )
    .with_clock(common::clock());
    let before = log.jsonl();
    let err = stale.update(&mut log, doc.clone()).unwrap_err();
    assert!(matches!(err, WebvhError::Refused(_)), "{err}");
    assert_eq!(log.jsonl(), before);
}

// @tg: verifies:: [[openspec:webvh-identity#Update And Rotate#Unannounced key refused]] {level: integration}
#[test]
fn unannounced_key_refused() {
    let keys = common::keys();
    let mut author = common::author(&keys, true);
    let options = CreateOptions::new("example.org").pre_rotation(true);
    let (_, doc) = common::document(&keys, &options, vec![]);
    let mut log = author.create(&options, &doc).unwrap();
    let rogue = keys.generate(KeyType::Ed25519).unwrap();
    let before = log.jsonl();
    let keys_before = author.update_keys().clone();
    let err = author.rotate(&mut log, Some(rogue)).unwrap_err();
    assert!(matches!(err, WebvhError::Refused(_)), "{err}");
    assert_eq!(log.jsonl(), before);
    assert_eq!(author.update_keys(), &keys_before);
}

// @tg: verifies:: [[openspec:webvh-identity#Deactivate#Deactivated DID]] {level: integration}
#[test]
fn deactivated_did() {
    for pre_rotation in [false, true] {
        let keys = common::keys();
        let mut author = common::author(&keys, pre_rotation);
        let options = CreateOptions::new("example.org").pre_rotation(pre_rotation);
        let (_, doc) = common::document(&keys, &options, vec![]);
        let mut log = author.create(&options, &doc).unwrap();
        author.deactivate(&mut log).unwrap();
        let v = log.verify().unwrap();
        assert!(v.deactivated, "pre_rotation={pre_rotation}");
        let err = author.update(&mut log, doc.clone()).unwrap_err();
        assert!(matches!(err, WebvhError::Refused(_)), "{err}");
    }
}

// @tg: verifies:: [[openspec:webvh-identity#Verify Before Publish#Broken log refused]] {level: integration}
#[test]
fn broken_log_refused() {
    let keys = common::keys();
    let mut author = common::author(&keys, false);
    let options = CreateOptions::new("example.org");
    let (_, doc) = common::document(&keys, &options, vec![]);
    let mut log = author.create(&options, &doc).unwrap();
    let new = with_service(log.document().unwrap(), "https://new.example/tsp");
    author.update(&mut log, new).unwrap();

    // Break the last entry's signature.
    let mut entries = log.entries().to_vec();
    let last = entries.last_mut().unwrap();
    let pv = last
        .pointer_mut("/proof/0/proofValue")
        .expect("a proof value");
    let s = pv.as_str().unwrap().to_owned();
    let flipped: String = s
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i == 5 {
                if c == 'A' { 'B' } else { 'A' }
            } else {
                c
            }
        })
        .collect();
    *pv = Value::String(flipped);
    let bad_version = last["versionId"].as_str().unwrap().to_owned();
    let jsonl: String = entries.iter().map(|e| format!("{e}\n")).collect();
    let broken = vertep_webvh::DidLog::from_files(&jsonl, None).unwrap();

    let site = tempfile::tempdir().unwrap();
    let err = DirectoryPublisher::new(site.path())
        .publish(&broken)
        .unwrap_err();
    match &err {
        WebvhError::Invalid { version_id, .. } => {
            assert_eq!(version_id.as_deref(), Some(bad_version.as_str()));
        }
        other => panic!("expected an invalid entry, got {other}"),
    }
    assert!(err.to_string().contains(&bad_version));
    assert_eq!(
        std::fs::read_dir(site.path()).unwrap().count(),
        0,
        "nothing written"
    );
}
