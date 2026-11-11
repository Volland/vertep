//! `did:webvh` resolution with log verification, offline through a canned fetcher.
#![cfg(feature = "webvh")]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::CannedHttp;
use didwebvh_rs::Multibase;
use didwebvh_rs::create::{CreateDIDConfig, create_did};
use didwebvh_rs::parameters::Parameters;
use didwebvh_rs::prelude::Secret;
use vertep_federation::{
    CommunicationFailure, Did, DidDirectory, DidResolver, ResolutionError, ResolverRegistry,
};

/// A one-entry did:webvh log for `https://example.com/agents/billing`, with a DIDComm service.
fn new_log() -> (String, String) {
    let mut key = Secret::generate_ed25519(None, None);
    let pk = key.get_public_keymultibase().expect("multibase");
    key.id = format!("did:key:{pk}#{pk}");
    let did_template = "did:webvh:{SCID}:example.com:agents:billing";
    let doc = serde_json::json!({
        "id": did_template,
        "@context": ["https://www.w3.org/ns/did/v1"],
        "verificationMethod": [{
            "id": format!("{did_template}#key-0"),
            "type": "Multikey",
            "publicKeyMultibase": pk,
            "controller": did_template
        }],
        "authentication": [format!("{did_template}#key-0")],
        "assertionMethod": [format!("{did_template}#key-0")],
        "service": [{
            "id": format!("{did_template}#didcomm"),
            "type": "DIDCommMessaging",
            "serviceEndpoint": {"uri": "https://mediator.example/inbox", "accept": ["didcomm/v2"]}
        }]
    });
    let params = Parameters {
        update_keys: Some(Arc::new(vec![Multibase::new(pk)])),
        ..Default::default()
    };
    let config = CreateDIDConfig::builder()
        .address("https://example.com/agents/billing")
        .authorization_key(key)
        .did_document(doc)
        .parameters(params)
        .build()
        .expect("config");
    let created = futures::executor::block_on(create_did(config)).expect("created offline");
    let log = serde_json::to_string(created.log_entry()).expect("log line");
    (created.did().to_owned(), log)
}

// @tg: verifies:: [[openspec:did-identity#Supported DID Methods#did:webvh resolves with history verification]] {level: integration}
#[test]
fn did_webvh_resolves_only_a_verified_log() {
    let (did, log) = new_log();
    let http = CannedHttp::new();
    http.serve("https://example.com/agents/billing/did.jsonl", log.clone());
    let registry = Arc::new(ResolverRegistry::with_builtin(
        Some(http.clone()),
        Duration::from_secs(1),
    ));
    let res = registry
        .resolve(&Did::parse(&did).expect("did"))
        .expect("verified log resolves");
    assert_eq!(res.document.id, did);
    assert!(
        res.version_id
            .as_deref()
            .is_some_and(|v| v.starts_with("1-"))
    );
    assert_eq!(http.count(), 1, "the log was fetched");
    let routes = DidDirectory::new(registry.clone())
        .resolve(&did)
        .expect("routes")
        .routes;
    assert_eq!(routes[0].endpoint.uri, "https://mediator.example/inbox");

    // A log whose entry was altered after signing does not resolve.
    let tampered = log.replace(
        "https://mediator.example/inbox",
        "https://evil.example/inbox",
    );
    assert_ne!(tampered, log);
    http.serve("https://example.com/agents/billing/did.jsonl", tampered);
    let err = registry
        .resolve(&Did::parse(&did).expect("did"))
        .expect_err("verification fails");
    assert!(
        matches!(err, ResolutionError::VerificationFailed(_)),
        "{err:?}"
    );
    let comm = DidDirectory::new(registry)
        .resolve(&did)
        .expect_err("no route");
    assert!(matches!(
        comm.failure,
        CommunicationFailure::ResolutionFailed(_)
    ));
}
