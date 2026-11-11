//! Key management, actor identities and key rotation.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use base64::Engine as _;
use oxrdf::{Literal, NamedNode, Triple};
use vertep_federation::credential::issue_grant_credential;
use vertep_federation::identity::KeyLookupError;
use vertep_federation::keys::{SealedBox, SharedSecret, seal_to};
use vertep_federation::ontology::{project_actor, project_inbound, validate};
use vertep_federation::{
    ActorIdentity, DidMethodChoice, FileKeyManager, IdentityConfig, InMemoryKeyManager,
    InboundRecord, KeyError, KeyManager, KeyType, PublicKey, Refusal, Service, ServiceEndpoint,
    Timestamp,
};

const SEED: [u8; 32] = [7u8; 32];

/// A key manager standing in for a hardware module: it holds keys elsewhere and counts every
/// private operation.
#[derive(Default)]
struct CountingHsm {
    inner: InMemoryKeyManager,
    signs: AtomicUsize,
    agreements: AtomicUsize,
    decrypts: AtomicUsize,
    generated: AtomicUsize,
}

impl KeyManager for CountingHsm {
    fn generate(&self, key_type: KeyType) -> Result<String, KeyError> {
        self.generated.fetch_add(1, Ordering::SeqCst);
        self.inner.generate(key_type)
    }
    fn public_key(&self, name: &str) -> Result<PublicKey, KeyError> {
        self.inner.public_key(name)
    }
    fn sign(&self, name: &str, data: &[u8]) -> Result<Vec<u8>, KeyError> {
        self.signs.fetch_add(1, Ordering::SeqCst);
        self.inner.sign(name, data)
    }
    fn key_agreement(&self, name: &str, peer: &PublicKey) -> Result<SharedSecret, KeyError> {
        self.agreements.fetch_add(1, Ordering::SeqCst);
        self.inner.key_agreement(name, peer)
    }
    fn decrypt(
        &self,
        name: &str,
        sealed: &SealedBox,
        info: &[u8],
        aad: &[u8],
    ) -> Result<Vec<u8>, KeyError> {
        self.decrypts.fetch_add(1, Ordering::SeqCst);
        self.inner.decrypt(name, sealed, info, aad)
    }
    fn delete(&self, name: &str) -> Result<(), KeyError> {
        self.inner.delete(name)
    }
}

fn secret_encodings(seed: &[u8; 32]) -> Vec<String> {
    vec![
        seed.iter().map(|b| format!("{b:02x}")).collect(),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(seed),
        base64::engine::general_purpose::STANDARD.encode(seed),
        bs58::encode(seed).into_string(),
    ]
}

fn hosted(did: &str) -> IdentityConfig {
    let mut c = IdentityConfig::new(DidMethodChoice::Hosted {
        did: did.into(),
        services: vec![Service {
            id: "#tsp".into(),
            types: vec!["TSPTransport".into()],
            endpoints: vec![ServiceEndpoint::uri("https://tsp.example/billing")],
        }],
    });
    c.rotation_grace = Duration::from_secs(3600);
    c
}

// @tg: verifies:: [[openspec:did-identity#Key Management Interface#Signing without exposing keys]] {level: integration}
#[test]
fn signing_goes_through_the_key_manager_and_identities_hold_no_keys() {
    let km = InMemoryKeyManager::new();
    let name = km.import_seed(KeyType::Ed25519, SEED).expect("import");
    let pk = km.public_key(&name).expect("pk");
    let did = format!("did:key:{}", pk.multikey());
    let identity = ActorIdentity {
        did: did.clone(),
        signing: vertep_federation::KeyRef {
            kid: format!("{did}#{}", pk.multikey()),
            key_name: name.clone(),
        },
        agreement: vertep_federation::KeyRef {
            kid: format!("{did}#{}", pk.to_x25519().expect("x").multikey()),
            key_name: name,
        },
        retired: vec![],
        services: vec![],
        rotatable: false,
        grace_ms: 0,
    };
    let grant = vertep_federation::Grant::new("did:example:alice", did.as_str(), ["order.*"]);
    let credential = issue_grant_credential(&km, &identity, &grant, Timestamp::from_secs(0))
        .expect("signed through the key manager");
    let proof = credential["proof"]["proofValue"].as_str().expect("proof");
    assert!(proof.starts_with('z'));

    // What a chart datamodel or journal could hold: the identity and the signed output.
    let stored =
        serde_json::to_string(&identity).expect("identity serializes") + &credential.to_string();
    for enc in secret_encodings(&SEED) {
        assert!(!stored.contains(&enc), "key material leaked as {enc}");
    }
    assert!(!format!("{km:?}").contains(&secret_encodings(&SEED)[0]));
}

// @tg: verifies:: [[openspec:did-identity#Key Management Interface#Pluggable key store]] {level: integration}
#[test]
fn every_private_operation_goes_through_a_pluggable_key_store() {
    let hsm = Arc::new(CountingHsm::default());
    let mut identity =
        ActorIdentity::provision(hsm.as_ref(), &hosted("did:web:example.com:billing"))
            .expect("provisioned in the HSM");
    assert_eq!(hsm.generated.load(Ordering::SeqCst), 2);

    let grant = vertep_federation::Grant::new("did:example:a", identity.did.as_str(), ["quote.*"]);
    issue_grant_credential(hsm.as_ref(), &identity, &grant, Timestamp::from_secs(0))
        .expect("signed");
    assert_eq!(hsm.signs.load(Ordering::SeqCst), 1);

    let enc = hsm.public_key(&identity.agreement.key_name).expect("pk");
    let sealed = seal_to(&enc, b"vertep", b"", b"secret").expect("sealed");
    let k = identity
        .decryption_key(&identity.agreement.kid, Timestamp::from_secs(0))
        .expect("current key");
    assert_eq!(
        hsm.decrypt(&k.key_name, &sealed, b"vertep", b"")
            .expect("opened"),
        b"secret"
    );
    assert_eq!(hsm.decrypts.load(Ordering::SeqCst), 1);

    identity
        .rotate(hsm.as_ref(), Timestamp::from_secs(10))
        .expect("rotation creates keys in the HSM");
    assert_eq!(hsm.generated.load(Ordering::SeqCst), 4);
}

// @tg: verifies:: [[openspec:did-identity#Key Management Interface#Graph contains only public keys]] {level: integration}
#[test]
fn exported_graph_contains_only_public_keys() {
    let km = InMemoryKeyManager::new();
    let identity =
        ActorIdentity::provision(&km, &hosted("did:web:example.com:billing")).expect("provisioned");
    let doc = identity.document(&km).expect("document");
    let graph = project_actor(&doc, Some("1-abc")).expect("projects");
    let report = validate(&graph);
    assert!(report.conforms(), "{report}");
    let text = graph.to_string();
    assert!(text.contains("publicKeyMultibase"));
    for vm in &doc.verification_method {
        assert!(text.contains(&vm.public_key.multikey()));
    }
    assert!(!text.to_lowercase().contains("private"));
    assert!(!text.to_lowercase().contains("secret"));

    // A private key property on a verification method makes the graph non-conforming.
    let mut leaky = graph.clone();
    leaky.insert(&Triple::new(
        NamedNode::new(&doc.verification_method[0].id).expect("iri"),
        NamedNode::new("https://w3id.org/security#secretKeyMultibase").expect("iri"),
        Literal::new_simple_literal("zSecret"),
    ));
    assert!(!validate(&leaky).conforms());
}

// @tg: verifies:: [[openspec:did-identity#Key Rotation Without Address Change#Rotation keeps the address]] {level: integration}
#[test]
fn rotation_keeps_the_address_and_senders_encrypt_to_new_keys() {
    let km = InMemoryKeyManager::new();
    let mut identity =
        ActorIdentity::provision(&km, &hosted("did:webvh:QmScid:example.com:billing"))
            .expect("provisioned");
    let before = identity.document(&km).expect("doc");
    let after = identity
        .rotate(&km, Timestamp::from_secs(100))
        .expect("rotated");
    assert_eq!(after.id, before.id, "the DID does not change");
    assert_eq!(identity.did, before.id);
    assert_ne!(after.key_agreement, before.key_agreement);
    assert_ne!(after.authentication, before.authentication);

    // A sender that re-resolves encrypts to the new key-agreement key.
    let new_key = after.key_agreement_methods()[0];
    let sealed = seal_to(&new_key.public_key, b"", b"", b"hello").expect("sealed");
    let k = identity
        .decryption_key(&new_key.id, Timestamp::from_secs(101))
        .expect("current key");
    assert_eq!(
        km.decrypt(&k.key_name, &sealed, b"", b"").expect("opens"),
        b"hello"
    );

    // did:key and did:peer bind their keys and cannot rotate.
    let mut bound =
        ActorIdentity::provision(&km, &IdentityConfig::new(DidMethodChoice::Key)).expect("did:key");
    assert!(bound.rotate(&km, Timestamp::from_secs(0)).is_err());
}

// @tg: verifies:: [[openspec:did-identity#Key Rotation Without Address Change#Message in flight during rotation]] {level: integration}
#[test]
fn message_in_flight_during_rotation_is_decrypted_within_the_grace_period() {
    let km = InMemoryKeyManager::new();
    let mut identity =
        ActorIdentity::provision(&km, &hosted("did:web:example.com:billing")).expect("provisioned");
    let old = identity.document(&km).expect("doc");
    let old_key = old.key_agreement_methods()[0].clone();
    let in_flight = seal_to(&old_key.public_key, b"", b"", b"order.placed").expect("sealed");

    identity
        .rotate(&km, Timestamp::from_secs(1_000))
        .expect("rotated");
    let now = Timestamp::from_secs(1_000 + 1_800); // half the one-hour grace period
    let k = identity
        .decryption_key(&old_key.id, now)
        .expect("retired key still usable");
    assert_eq!(
        km.decrypt(&k.key_name, &in_flight, b"", b"")
            .expect("delivered"),
        b"order.placed"
    );
}

// @tg: verifies:: [[openspec:did-identity#Key Rotation Without Address Change#Message after the grace period]] {level: integration}
#[test]
fn message_after_the_grace_period_is_recorded_as_undecryptable() {
    let km = InMemoryKeyManager::new();
    let mut identity =
        ActorIdentity::provision(&km, &hosted("did:web:example.com:billing")).expect("provisioned");
    let old_kid = identity.agreement.kid.clone();
    identity
        .rotate(&km, Timestamp::from_secs(1_000))
        .expect("rotated");
    let late = Timestamp::from_secs(1_000 + 3_601);
    let err = identity
        .decryption_key(&old_kid, late)
        .expect_err("retired beyond grace");
    assert!(matches!(err, KeyLookupError::RetiredBeyondGrace { .. }));
    identity.purge_retired(&km, late).expect("purged");
    assert!(!km.contains(&identity.retired[0].key.key_name));

    // Not delivered; recorded with its sender.
    let record = InboundRecord::refused(
        &identity.did,
        None,
        Refusal::Undecryptable {
            reason: err.to_string(),
            kid: Some(old_kid),
            sender: Some("did:example:sender".into()),
        },
        late,
    );
    assert!(record.refusal.is_some());
    let g = project_inbound(&record).expect("projects");
    let report = validate(&g);
    assert!(report.conforms(), "{report}");
    assert!(g.to_string().contains("past the grace period"));
}

#[test]
fn file_key_manager_persists_keys_privately() {
    let dir = tempfile::tempdir().expect("tempdir");
    let name = {
        let km = FileKeyManager::open(dir.path()).expect("open");
        km.generate(KeyType::Ed25519).expect("generate")
    };
    let km = FileKeyManager::open(dir.path()).expect("reopen");
    let pk = km.public_key(&name).expect("persisted");
    let sig = km.sign(&name, b"msg").expect("sign");
    assert!(pk.verify(b"msg", &sig));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let meta = std::fs::metadata(dir.path().join(format!("{name}.key"))).expect("meta");
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
    assert!(matches!(
        km.sign("../etc", b""),
        Err(KeyError::UnknownKey(_))
    ));
    km.delete(&name).expect("delete");
    assert!(!km.contains(&name));
}

#[test]
fn per_actor_type_configuration_gives_each_instance_a_did_address() {
    let km = InMemoryKeyManager::new();
    let config = IdentityConfig::new(DidMethodChoice::Peer {
        services: vec![Service {
            id: "#didcomm".into(),
            types: vec!["DIDCommMessaging".into()],
            endpoints: vec![ServiceEndpoint::uri("https://mediator.example/")],
        }],
    });
    let a = ActorIdentity::provision(&km, &config).expect("a");
    let b = ActorIdentity::provision(&km, &config).expect("b");
    assert!(a.did.starts_with("did:peer:2."));
    assert_ne!(a.did, b.did, "each instance gets its own DID");
    let doc = a.document(&km).expect("doc");
    assert_eq!(doc.id, a.did);
    assert_eq!(doc.service[0].id, format!("{}#didcomm", a.did));
}
