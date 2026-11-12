//! DIDComm v2: authcrypt and anoncrypt through the key manager, forward routing through
//! mediators, opening, and wire compatibility with Affinidi's DIDComm implementation.
#![cfg(feature = "didcomm")]

use std::sync::Arc;
use std::time::Duration;

use affinidi_crypto::jose::key_agreement::{Curve, PrivateKeyAgreement, PublicKeyAgreement};
use affinidi_messaging_didcomm::SenderKey;
use affinidi_messaging_didcomm::jwe::decrypt::decrypt_bound;
use affinidi_messaging_didcomm::jwe::encrypt::authcrypt as affinidi_authcrypt;
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_core::types::parse_declarations;
use vertep_federation::didcomm::{DidCommAgent, DidCommError, DidCommReceived, agreement_keys};
use vertep_federation::ontology::{project_inbound, validate};
use vertep_federation::origin::DeclaredPayloads;
use vertep_federation::resolver::peer::{PeerPurpose, did_peer_2};
use vertep_federation::vocab::DIDCOMM_PROCESSOR;
use vertep_federation::{
    ActorIdentity, DidDirectory, DidMethodChoice, DidResolver, Grant, GrantSet, IdentityConfig,
    InMemoryKeyManager, InboundPolicy, InboundRecord, KeyManager, KeyRef, KeyType, OpenedMessage,
    RelayEnvelope, ResolverRegistry, Service, ServiceEndpoint, Timestamp, admit,
};

fn registry() -> ResolverRegistry {
    ResolverRegistry::with_builtin(None, Duration::from_secs(1))
}

fn km() -> Arc<InMemoryKeyManager> {
    Arc::new(InMemoryKeyManager::new())
}

fn dm_service(uri: &str, routing_keys: &[String]) -> Service {
    Service {
        id: "#didcomm".into(),
        types: vec!["DIDCommMessaging".into()],
        endpoints: vec![ServiceEndpoint {
            uri: uri.into(),
            routing_keys: routing_keys.to_vec(),
            accept: vec!["didcomm/v2".into()],
        }],
    }
}

fn peer(km: &Arc<InMemoryKeyManager>, uri: &str, routing_keys: &[String]) -> DidCommAgent {
    let id = ActorIdentity::provision(
        km.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![dm_service(uri, routing_keys)],
        }),
    )
    .expect("identity");
    DidCommAgent::new(id, km.clone() as Arc<dyn KeyManager>)
}

fn mediator(km: &Arc<InMemoryKeyManager>) -> DidCommAgent {
    let id = ActorIdentity::provision(km.as_ref(), &IdentityConfig::new(DidMethodChoice::Key))
        .expect("mediator");
    DidCommAgent::new(id, km.clone() as Arc<dyn KeyManager>)
}

fn order(send_id: &str) -> Event {
    let mut e = Event::named("order.placed", EventType::External);
    e.sendid = Some(send_id.into());
    e.data = Value::Object(vec![
        ("id".into(), Value::String("secret-order-77".into())),
        ("amount".into(), Value::Number(42.0)),
    ]);
    e
}

fn doc_of(agent: &DidCommAgent, km: &Arc<InMemoryKeyManager>) -> vertep_federation::DidDocument {
    agent.identity().document(km.as_ref()).expect("doc")
}

fn event_of(r: DidCommReceived) -> OpenedMessage {
    match r {
        DidCommReceived::Event(m) => *m,
        other => panic!("expected an event, got {other:?}"),
    }
}

fn forward_of(r: DidCommReceived) -> (String, String) {
    match r {
        DidCommReceived::Forward { next, message } => (next, message),
        other => panic!("expected a forward, got {other:?}"),
    }
}

// @tg: verifies:: [[openspec:relay-mailboxes#SCXML Event Envelope Mapping#Round trip of an event]] {level: integration}
#[test]
fn round_trip_of_an_event_over_didcomm() {
    let km = km();
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let b = peer(&km, "https://b.example/didcomm", &[]);
    let reg = registry();
    let b_doc = doc_of(&b, &km);
    let route = DidDirectory::new(Arc::new(registry()))
        .resolve(&b.identity().did)
        .expect("resolves")
        .routes[0]
        .clone();
    let out = a
        .seal_event(
            &b_doc,
            &route,
            &RelayEnvelope::from_event("m-1", &order("s-17")),
            false,
            &reg,
        )
        .expect("sealed");
    assert_eq!(out.endpoint, "https://b.example/didcomm");
    let opened = event_of(
        b.open(&out.message, &reg, Timestamp::from_secs(0))
            .expect("opens"),
    );

    let types = DeclaredPayloads::new(
        parse_declarations("event order.placed: { id: string, amount: decimal };").expect("decls"),
    );
    let mut grants = GrantSet::new();
    grants.add(Grant::new(
        a.identity().did.as_str(),
        b.identity().did.as_str(),
        ["order.*"],
    ));
    let policy = InboundPolicy {
        actor: b.identity().did.clone(),
        anonymous: Default::default(),
        grants,
    };
    let admitted = admit(&policy, &opened, &types, Timestamp::from_secs(0)).expect("admitted");
    let ev = admitted.event;
    assert_eq!(ev.name, "order.placed");
    assert_eq!(ev.data, order("s-17").data);
    assert_eq!(ev.sendid.as_deref(), Some("s-17"));
    assert_eq!(ev.origin.as_deref(), Some(a.identity().did.as_str()));
    assert_eq!(ev.origintype.as_deref(), Some(DIDCOMM_PROCESSOR));
    assert_eq!(admitted.message_id, "m-1");
}

// @tg: verifies:: [[openspec:did-identity#Authenticated Event Origin#Verified sender]] {level: integration}
#[test]
fn verified_sender_from_a_did_peer() {
    let km = km();
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let b = peer(&km, "https://b.example/didcomm", &[]);
    assert!(a.identity().did.starts_with("did:peer:2."));
    let reg = registry();
    let jwe = a
        .authcrypt(
            &serde_json::to_vec(&serde_json::json!({
                "id": "m-2", "type": vertep_federation::vocab::DIDCOMM_EVENT_TYPE,
                "from": a.identity().did, "to": [b.identity().did],
                "body": RelayEnvelope::from_event("m-2", &Event::named("approve", EventType::External)),
            }))
            .expect("json"),
            &agreement_keys(&doc_of(&b, &km)),
        )
        .expect("authcrypt");
    let opened = event_of(b.open(&jwe, &reg, Timestamp::from_secs(0)).expect("opens"));
    assert!(opened.origin.verified);
    assert_eq!(
        opened.origin.sender.as_deref(),
        Some(a.identity().did.as_str())
    );
    assert_eq!(
        opened.origin.sender_key.as_deref(),
        Some(a.identity().agreement.kid.as_str())
    );
    let mut ev = opened.envelope.to_event();
    opened.origin.apply(&mut ev);
    assert_eq!(ev.origin.as_deref(), Some(a.identity().did.as_str()));
    assert!(ev.originverified, "_event marks the origin as verified");
}

// @tg: verifies:: [[openspec:did-identity#Authenticated Event Origin#Forged origin claim]] {level: integration}
#[test]
fn forged_origin_claim_is_rejected_and_recorded() {
    let km = km();
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let b = peer(&km, "https://b.example/didcomm", &[]);
    // Mallory claims A's key id but holds only her own key.
    let mallory_key = km.generate(KeyType::X25519).expect("mallory");
    let mut forged_id = a.identity().clone();
    forged_id.agreement = KeyRef {
        kid: a.identity().agreement.kid.clone(),
        key_name: mallory_key,
    };
    let mallory = DidCommAgent::new(forged_id, km.clone() as Arc<dyn KeyManager>);
    let reg = registry();
    let b_doc = doc_of(&b, &km);
    let route = DidDirectory::new(Arc::new(registry()))
        .resolve(&b.identity().did)
        .expect("resolves")
        .routes[0]
        .clone();
    let out = mallory
        .seal_event(
            &b_doc,
            &route,
            &RelayEnvelope::from_event("m-3", &order("s-1")),
            false,
            &reg,
        )
        .expect("sealed");
    let err = b
        .open(&out.message, &reg, Timestamp::from_secs(0))
        .expect_err("not delivered as coming from A");
    assert!(
        matches!(&err, DidCommError::ForgedOrigin { claimed, .. } if *claimed == a.identity().did),
        "{err:?}"
    );
    let record = InboundRecord::refused(
        &b.identity().did,
        None,
        err.to_refusal(),
        Timestamp::from_secs(0),
    );
    let g = project_inbound(&record).expect("recorded");
    assert!(validate(&g).conforms());
    assert!(g.to_string().contains("not authenticated"));
}

#[test]
fn anoncrypt_has_no_verified_origin_even_if_from_is_claimed() {
    let km = km();
    let b = peer(&km, "https://b.example/didcomm", &[]);
    let reg = registry();
    let plaintext = serde_json::to_vec(&serde_json::json!({
        "id": "m-4", "type": vertep_federation::vocab::DIDCOMM_EVENT_TYPE,
        "from": "did:example:claimed", "to": [b.identity().did],
        "body": RelayEnvelope::from_event("m-4", &Event::named("ping", EventType::External)),
    }))
    .expect("json");
    let jwe = vertep_federation::didcomm::anoncrypt(&plaintext, &agreement_keys(&doc_of(&b, &km)))
        .expect("anoncrypt");
    let opened = event_of(b.open(&jwe, &reg, Timestamp::from_secs(0)).expect("opens"));
    assert!(!opened.origin.verified);
    assert_eq!(opened.origin.sender, None);
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Forward Routing#One mediator]] {level: integration}
#[test]
fn one_mediator_forward() {
    let km = km();
    let m = mediator(&km);
    let b = peer(
        &km,
        "https://mediator.example/inbound",
        std::slice::from_ref(&m.identity().agreement.kid),
    );
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let reg = registry();
    let b_doc = doc_of(&b, &km);
    let resolved = DidDirectory::new(Arc::new(registry()))
        .resolve(&b.identity().did)
        .expect("resolves");
    assert!(resolved.routes[0].is_mediated());
    let out = a
        .seal_event(
            &b_doc,
            &resolved.routes[0],
            &RelayEnvelope::from_event("m-5", &order("s-5")),
            false,
            &reg,
        )
        .expect("sealed");
    assert_eq!(
        out.endpoint, "https://mediator.example/inbound",
        "posted to the mediator"
    );
    let (next, inner) = forward_of(
        m.open(&out.message, &reg, Timestamp::from_secs(0))
            .expect("mediator opens its layer"),
    );
    assert_eq!(next, b.identity().did);
    let opened = event_of(
        b.open(&inner, &reg, Timestamp::from_secs(0))
            .expect("recipient opens"),
    );
    assert_eq!(opened.envelope.name, "order.placed");
    assert_eq!(
        opened.origin.sender.as_deref(),
        Some(a.identity().did.as_str())
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#DIDComm Forward Routing#Chain of mediators]] {level: integration}
#[test]
fn chain_of_mediators_each_unwraps_only_its_layer() {
    let km = km();
    let m1 = mediator(&km);
    let m2 = mediator(&km);
    let routing = vec![
        m1.identity().agreement.kid.clone(),
        m2.identity().agreement.kid.clone(),
    ];
    let b = peer(&km, "https://m1.example/inbound", &routing);
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let reg = registry();
    let resolved = DidDirectory::new(Arc::new(registry()))
        .resolve(&b.identity().did)
        .expect("resolves");
    let out = a
        .seal_event(
            &doc_of(&b, &km),
            &resolved.routes[0],
            &RelayEnvelope::from_event("m-6", &order("s-6")),
            false,
            &reg,
        )
        .expect("sealed");
    let now = Timestamp::from_secs(0);
    // Only m1 opens the outer layer.
    assert_eq!(
        m2.open(&out.message, &reg, now),
        Err(DidCommError::NotForMe)
    );
    assert_eq!(b.open(&out.message, &reg, now), Err(DidCommError::NotForMe));
    let (next1, layer2) = forward_of(m1.open(&out.message, &reg, now).expect("m1"));
    assert_eq!(next1, m2.identity().agreement.kid);
    // m1 cannot open the next layer; m2 can.
    assert_eq!(m1.open(&layer2, &reg, now), Err(DidCommError::NotForMe));
    let (next2, layer3) = forward_of(m2.open(&layer2, &reg, now).expect("m2"));
    assert_eq!(next2, b.identity().did);
    assert_eq!(m2.open(&layer3, &reg, now), Err(DidCommError::NotForMe));
    let opened = event_of(b.open(&layer3, &reg, now).expect("b"));
    assert_eq!(opened.envelope.message_id, "m-6");
}

// @tg: verifies:: [[openspec:relay-mailboxes#End-To-End Encryption Through Relays#Mediator sees only routing data]] {level: integration}
#[test]
fn mediator_sees_only_routing_data() {
    let km = km();
    let m = mediator(&km);
    let b = peer(
        &km,
        "https://mediator.example/inbound",
        std::slice::from_ref(&m.identity().agreement.kid),
    );
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let reg = registry();
    let resolved = DidDirectory::new(Arc::new(registry()))
        .resolve(&b.identity().did)
        .expect("resolves");
    let out = a
        .seal_event(
            &doc_of(&b, &km),
            &resolved.routes[0],
            &RelayEnvelope::from_event("m-7", &order("s-77")),
            false,
            &reg,
        )
        .expect("sealed");
    let (next, stored) = forward_of(
        m.open(&out.message, &reg, Timestamp::from_secs(0))
            .expect("opens"),
    );
    // What the mediator holds: the next hop and the recipient's ciphertext.
    assert_eq!(next, b.identity().did);
    let jwe: serde_json::Value = serde_json::from_str(&stored).expect("jwe");
    assert_eq!(
        jwe["recipients"][0]["header"]["kid"].as_str(),
        Some(b.identity().agreement.kid.as_str()),
        "the recipient routing key"
    );
    use base64::Engine as _;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(jwe["protected"].as_str().expect("protected"))
        .expect("b64");
    let visible = format!("{}{}", out.message, stored) + &String::from_utf8_lossy(&header);
    for secret in ["order.placed", "secret-order-77", "s-77", "m-7"] {
        assert!(
            !visible.contains(secret),
            "{secret} visible to the mediator"
        );
    }
    assert_eq!(
        m.open(&stored, &reg, Timestamp::from_secs(0)),
        Err(DidCommError::NotForMe)
    );
}

// @tg: verifies:: [[openspec:relay-mailboxes#End-To-End Encryption Through Relays#Tampered message]] {level: integration}
#[test]
fn tampered_message_is_not_delivered_and_is_recorded() {
    let km = km();
    let m = mediator(&km);
    let b = peer(
        &km,
        "https://mediator.example/inbound",
        std::slice::from_ref(&m.identity().agreement.kid),
    );
    let a = peer(&km, "https://a.example/didcomm", &[]);
    let reg = registry();
    let resolved = DidDirectory::new(Arc::new(registry()))
        .resolve(&b.identity().did)
        .expect("resolves");
    let out = a
        .seal_event(
            &doc_of(&b, &km),
            &resolved.routes[0],
            &RelayEnvelope::from_event("m-8", &order("s-8")),
            false,
            &reg,
        )
        .expect("sealed");
    let (_, stored) = forward_of(
        m.open(&out.message, &reg, Timestamp::from_secs(0))
            .expect("opens"),
    );
    // The relay flips one ciphertext character.
    let mut jwe: serde_json::Value = serde_json::from_str(&stored).expect("jwe");
    let ct = jwe["ciphertext"].as_str().expect("ct").to_owned();
    let flipped: String = ct
        .char_indices()
        .map(|(i, c)| {
            if i == 5 {
                if c == 'A' { 'B' } else { 'A' }
            } else {
                c
            }
        })
        .collect();
    jwe["ciphertext"] = serde_json::json!(flipped);
    let err = b
        .open(&jwe.to_string(), &reg, Timestamp::from_secs(0))
        .expect_err("fails to decrypt or authenticate");
    assert!(matches!(err, DidCommError::Undecryptable { .. }), "{err:?}");
    let record = InboundRecord::refused(
        &b.identity().did,
        None,
        err.to_refusal(),
        Timestamp::from_secs(0),
    );
    assert!(validate(&project_inbound(&record).expect("recorded")).conforms());
}

/// A did:peer:2 identity from known seeds, so the test can also hand raw keys to Affinidi.
fn seeded(km: &Arc<InMemoryKeyManager>, sig: u8, enc: u8) -> (DidCommAgent, [u8; 32]) {
    let s = km.import_seed(KeyType::Ed25519, [sig; 32]).expect("sig");
    let e = km.import_seed(KeyType::X25519, [enc; 32]).expect("enc");
    let did = did_peer_2(
        &[
            (PeerPurpose::Verification, km.public_key(&s).expect("s")),
            (PeerPurpose::Encryption, km.public_key(&e).expect("e")),
        ],
        &[],
    );
    let id = ActorIdentity {
        signing: KeyRef {
            kid: format!("{did}#key-1"),
            key_name: s,
        },
        agreement: KeyRef {
            kid: format!("{did}#key-2"),
            key_name: e,
        },
        did,
        retired: vec![],
        services: vec![],
        rotatable: false,
        grace_ms: 0,
    };
    (
        DidCommAgent::new(id, km.clone() as Arc<dyn KeyManager>),
        [enc; 32],
    )
}

#[test]
fn wire_compatible_with_affinidi_in_both_directions() {
    let km = km();
    let (a, a_enc) = seeded(&km, 1, 2);
    let (b, b_enc) = seeded(&km, 3, 4);
    let reg = registry();
    let a_priv = PrivateKeyAgreement::from_raw_bytes(Curve::X25519, &a_enc).expect("a");
    let b_priv = PrivateKeyAgreement::from_raw_bytes(Curve::X25519, &b_enc).expect("b");
    let a_pub = a_priv.public_key();
    let b_pub = b_priv.public_key();
    let a_kid = a.identity().agreement.kid.clone();
    let b_kid = b.identity().agreement.kid.clone();
    let b_doc = doc_of(&b, &km);
    assert_eq!(
        agreement_keys(&b_doc)[0].1.bytes.to_vec(),
        match &b_pub {
            PublicKeyAgreement::X25519(x) => x.to_vec(),
            _ => panic!("x25519"),
        }
    );

    // Ours to theirs.
    let jwe = a
        .authcrypt(b"hello affinidi", &agreement_keys(&b_doc))
        .expect("ours");
    let dec = decrypt_bound(&jwe, &b_kid, &b_priv, Some(SenderKey::new(&a_kid, &a_pub)))
        .expect("Affinidi decrypts our authcrypt");
    assert_eq!(dec.plaintext, b"hello affinidi");
    assert!(dec.authenticated);

    // Theirs to ours.
    let msg = serde_json::to_vec(&serde_json::json!({
        "id": "m-9", "type": vertep_federation::vocab::DIDCOMM_EVENT_TYPE,
        "from": a.identity().did, "to": [b.identity().did],
        "body": RelayEnvelope::from_event("m-9", &Event::named("ping", EventType::External)),
    }))
    .expect("json");
    let theirs = affinidi_authcrypt(&msg, &a_kid, &a_priv, &[(b_kid.as_str(), &b_pub)])
        .expect("Affinidi authcrypt");
    let opened = event_of(
        b.open(&theirs, &reg, Timestamp::from_secs(0))
            .expect("we open theirs"),
    );
    assert_eq!(
        opened.origin.sender.as_deref(),
        Some(a.identity().did.as_str())
    );
    let _ = reg.resolve(&vertep_federation::Did::parse(&a.identity().did).expect("did"));
}
