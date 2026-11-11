//! DID methods and the pluggable resolver registry.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{CannedHttp, StaticResolver};
use vertep_federation::resolver::FetchError;
use vertep_federation::{
    CommunicationFailure, Did, DidDirectory, DidDocument, DidResolver, PublicKey, ResolutionError,
    ResolverRegistry, Service, ServiceEndpoint, VerificationMethod,
};

const MK: &str = "z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

fn doc_with_dm(did: &str) -> DidDocument {
    let key = PublicKey::from_multikey(MK).expect("key");
    let mut d = DidDocument::new(did);
    let vm = format!("{did}#k");
    d.verification_method.push(VerificationMethod::multikey(
        &vm,
        did,
        key.to_x25519().expect("x"),
    ));
    d.key_agreement.push(vm);
    d.service.push(Service {
        id: format!("{did}#dm"),
        types: vec!["DIDCommMessaging".into()],
        endpoints: vec![ServiceEndpoint::uri("https://mediator.example/")],
    });
    d
}

// @tg: verifies:: [[openspec:did-identity#Supported DID Methods#did:key resolves offline]] {level: integration}
#[test]
fn did_key_resolves_offline() {
    // No HTTP fetcher at all: did:key cannot touch the network.
    let registry = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    let did = Did::parse(&format!("did:key:{MK}")).expect("did");
    let doc = registry.resolve(&did).expect("resolves").document;
    assert_eq!(doc.id, did.as_str());
    assert_eq!(doc.authentication, vec![format!("did:key:{MK}#{MK}")]);
    let ka = doc.key_agreement_methods();
    assert_eq!(ka.len(), 1);
    assert_eq!(
        ka[0].public_key.multikey(),
        "z6LSj72tK8brWgZja8NLRwPigth2T9QRiG1uH9oKZuKjdh9p"
    );
}

// @tg: verifies:: [[openspec:did-identity#Supported DID Methods#Unsupported method]] {level: integration}
#[test]
fn unsupported_method_is_a_communication_error_naming_it() {
    let registry = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let dir = DidDirectory::new(registry);
    let err = dir
        .resolve("did:example:123")
        .expect_err("no resolver for did:example");
    assert_eq!(
        err.failure,
        CommunicationFailure::UnsupportedMethod("example".into())
    );
    let ev = err.for_send("s-1").to_event();
    assert_eq!(ev.name, "error.communication");
    assert_eq!(ev.sendid.as_deref(), Some("s-1"));
    let msg = format!("{:?}", ev.data);
    assert!(msg.contains("did:example"), "{msg}");
}

// @tg: verifies:: [[openspec:did-identity#Pluggable DID Resolver#Application registers a custom method]] {level: integration}
#[test]
fn application_registers_a_custom_method() {
    let registry = Arc::new(ResolverRegistry::with_builtin(None, Duration::from_secs(1)));
    let custom = StaticResolver::new();
    custom.put(doc_with_dm("did:example:abc"));
    registry.register("example", custom.clone());
    let dir = DidDirectory::new(registry);
    let resolved = dir
        .resolve("did:example:abc")
        .expect("resolved by the custom method");
    assert_eq!(resolved.routes[0].endpoint.uri, "https://mediator.example/");
    assert_eq!(custom.calls(), 1);
}

// @tg: verifies:: [[openspec:did-identity#Pluggable DID Resolver#Application overrides a built-in method]] {level: integration}
#[test]
fn application_overrides_a_built_in_method() {
    let http = CannedHttp::new();
    let registry = ResolverRegistry::with_builtin(Some(http.clone()), Duration::from_secs(1));
    let own = StaticResolver::new();
    own.put(doc_with_dm("did:web:example.com"));
    own.put(doc_with_dm("did:web:example.com:a"));
    registry.register("web", own.clone());
    for d in ["did:web:example.com", "did:web:example.com:a"] {
        registry
            .resolve(&Did::parse(d).expect("did"))
            .expect("the override resolves");
    }
    assert_eq!(own.calls(), 2, "every did:web address used the override");
    assert_eq!(http.count(), 0, "the built-in resolver was never called");
}

#[test]
fn did_web_fetches_through_the_injected_fetcher() {
    let http = CannedHttp::new();
    let doc = doc_with_dm("did:web:example.com%3A8443:org:billing");
    http.serve(
        "https://example.com:8443/org/billing/did.json",
        serde_json::to_vec(&doc.to_json()).expect("json"),
    );
    let registry = ResolverRegistry::with_builtin(Some(http.clone()), Duration::from_secs(1));
    let got = registry
        .resolve(&Did::parse(&doc.id).expect("did"))
        .expect("resolves")
        .document;
    assert_eq!(got, doc);

    http.fail(
        "https://example.com/.well-known/did.json",
        FetchError::Timeout,
    );
    let err = registry
        .resolve(&Did::parse("did:web:example.com").expect("did"))
        .expect_err("times out");
    assert!(matches!(err, ResolutionError::Timeout(_)));
}

#[test]
fn did_web_refuses_a_document_for_another_did() {
    let http = CannedHttp::new();
    http.serve(
        "https://evil.example/.well-known/did.json",
        serde_json::to_vec(&doc_with_dm("did:web:victim.example").to_json()).expect("json"),
    );
    let registry = ResolverRegistry::with_builtin(Some(http), Duration::from_secs(1));
    let err = registry
        .resolve(&Did::parse("did:web:evil.example").expect("did"))
        .expect_err("mismatched id");
    assert!(matches!(err, ResolutionError::InvalidDocument(_)));
}

#[test]
fn did_peer_0_matches_did_key() {
    let registry = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    let peer = registry
        .resolve(&Did::parse(&format!("did:peer:0{MK}")).expect("did"))
        .expect("resolves")
        .document;
    let key = registry
        .resolve(&Did::parse(&format!("did:key:{MK}")).expect("did"))
        .expect("resolves")
        .document;
    assert_eq!(
        peer.verification_method[0].public_key,
        key.verification_method[0].public_key
    );
}
