//! DID resolution as address resolution, and the resolution cache.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{CannedHttp, StaticResolver};
use vertep_federation::resolver::{AttemptError, DeliverError, FetchError};
use vertep_federation::{
    CachingResolver, CommunicationFailure, Did, DidDirectory, DidDocument, DidResolver,
    ManualClock, PublicKey, ResolverRegistry, Service, ServiceEndpoint, Timestamp, TransportKind,
    VerificationMethod,
};

const MK: &str = "z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

fn doc(did: &str, services: Vec<Service>) -> DidDocument {
    let key = PublicKey::from_multikey(MK).expect("key");
    let mut d = DidDocument::new(did);
    d.verification_method
        .push(VerificationMethod::multikey(format!("{did}#a"), did, key));
    d.verification_method.push(VerificationMethod::multikey(
        format!("{did}#e"),
        did,
        key.to_x25519().expect("x"),
    ));
    d.authentication.push(format!("{did}#a"));
    d.key_agreement.push(format!("{did}#e"));
    d.service = services;
    d
}

fn svc(did: &str, frag: &str, ty: &str, uri: &str, routing: &[&str]) -> Service {
    Service {
        id: format!("{did}#{frag}"),
        types: vec![ty.into()],
        endpoints: vec![ServiceEndpoint {
            uri: uri.into(),
            routing_keys: routing.iter().map(|s| (*s).to_owned()).collect(),
            accept: vec!["didcomm/v2".into()],
        }],
    }
}

// @tg: verifies:: [[openspec:did-identity#DID Resolution As Address Resolution#Service endpoint becomes a mailbox]] {level: integration}
#[test]
fn service_endpoint_becomes_a_mailbox_route() {
    let did = "did:example:bob";
    let r = StaticResolver::new();
    r.put(doc(
        did,
        vec![svc(
            did,
            "dm",
            "DIDCommMessaging",
            "https://mediator.example/inbox",
            &["did:example:mediator#key-x"],
        )],
    ));
    let dir = DidDirectory::new(r);
    let resolved = dir.resolve(did).expect("resolves");
    assert_eq!(resolved.routes.len(), 1);
    let route = &resolved.routes[0];
    assert_eq!(route.transport, TransportKind::DidComm);
    assert_eq!(route.endpoint.uri, "https://mediator.example/inbox");
    assert!(route.is_mediated());
    assert_eq!(resolved.encryption_keys().len(), 1);
    assert_eq!(resolved.authentication_keys().len(), 1);
}

// @tg: verifies:: [[openspec:did-identity#DID Resolution As Address Resolution#No usable service entry]] {level: integration}
#[test]
fn no_usable_service_entry() {
    let did = "did:example:carol";
    let r = StaticResolver::new();
    r.put(doc(
        did,
        vec![svc(
            did,
            "web",
            "LinkedDomains",
            "https://carol.example",
            &[],
        )],
    ));
    let err = DidDirectory::new(r).resolve(did).expect_err("no transport");
    assert_eq!(err.failure, CommunicationFailure::NoCompatibleEndpoint);
    assert!(err.to_event().name == "error.communication");
    assert!(format!("{:?}", err.to_event().data).contains("no compatible endpoint"));
}

#[test]
fn resolution_timeout_is_reported_once_not_retried() {
    let http = CannedHttp::new();
    http.fail(
        "https://slow.example/.well-known/did.json",
        FetchError::Timeout,
    );
    let registry = Arc::new(ResolverRegistry::with_builtin(
        Some(http.clone()),
        Duration::from_millis(50),
    ));
    let err = DidDirectory::new(registry)
        .resolve("did:web:slow.example")
        .expect_err("times out");
    assert!(matches!(
        err.failure,
        CommunicationFailure::ResolutionFailed(_)
    ));
    assert_eq!(
        err.for_send("s-9").to_event().sendid.as_deref(),
        Some("s-9")
    );
    assert_eq!(http.count(), 1);
}

#[test]
fn transports_are_tried_in_preference_order() {
    let did = "did:example:dora";
    let r = StaticResolver::new();
    r.put(doc(
        did,
        vec![
            svc(did, "dm", "DIDCommMessaging", "https://m.example", &[]),
            svc(did, "tsp", "TSPTransport", "https://tsp.example", &[]),
        ],
    ));
    let default = DidDirectory::new(r.clone()).resolve(did).expect("resolves");
    assert_eq!(default.routes[0].transport, TransportKind::Tsp);
    assert_eq!(default.routes[1].transport, TransportKind::DidComm);
    let didcomm_first = DidDirectory::new(r)
        .with_preference(vec![TransportKind::DidComm, TransportKind::Tsp])
        .resolve(did)
        .expect("resolves");
    assert_eq!(didcomm_first.routes[0].transport, TransportKind::DidComm);
}

// @tg: verifies:: [[openspec:did-identity#Resolution Caching#Cached document is reused]] {level: integration}
#[test]
fn cached_document_is_reused_within_its_lifetime() {
    let did = Did::parse("did:example:erin").expect("did");
    let inner = StaticResolver::new();
    inner.put(doc(did.as_str(), vec![]));
    let clock = Arc::new(ManualClock::new(Timestamp::from_secs(1_000)));
    let cache = CachingResolver::new(inner.clone(), Duration::from_secs(60), 16, clock.clone());
    cache.resolve(&did).expect("first");
    clock.advance(Duration::from_secs(30));
    cache.resolve(&did).expect("second");
    assert_eq!(inner.calls(), 1, "resolved once within the lifetime");
    clock.advance(Duration::from_secs(31));
    cache.resolve(&did).expect("after expiry");
    assert_eq!(inner.calls(), 2, "re-resolved on expiry");
}

#[test]
fn cache_is_bounded() {
    let inner = StaticResolver::new();
    for i in 0..5 {
        inner.put(doc(&format!("did:example:n{i}"), vec![]));
    }
    let clock = Arc::new(ManualClock::new(Timestamp::from_secs(0)));
    let cache = CachingResolver::new(inner.clone(), Duration::from_secs(600), 2, clock.clone());
    for i in 0..5 {
        clock.advance(Duration::from_secs(1));
        cache
            .resolve(&Did::parse(&format!("did:example:n{i}")).expect("did"))
            .expect("resolves");
    }
    // The two most recent are cached; the first was evicted.
    cache
        .resolve(&Did::parse("did:example:n4").expect("did"))
        .expect("cached");
    assert_eq!(inner.calls(), 5);
    cache
        .resolve(&Did::parse("did:example:n0").expect("did"))
        .expect("evicted, resolved again");
    assert_eq!(inner.calls(), 6);
}

// @tg: verifies:: [[openspec:did-identity#Resolution Caching#Stale endpoint triggers re-resolution]] {level: integration}
#[test]
fn stale_endpoint_triggers_one_re_resolution_and_retry() {
    let did = Did::parse("did:example:finn").expect("did");
    let inner = StaticResolver::new();
    inner.put(doc(
        did.as_str(),
        vec![svc(
            did.as_str(),
            "dm",
            "DIDCommMessaging",
            "https://old.example",
            &[],
        )],
    ));
    let clock = Arc::new(ManualClock::new(Timestamp::from_secs(0)));
    let cache = CachingResolver::new(inner.clone(), Duration::from_secs(600), 16, clock);
    cache.resolve(&did).expect("warm the cache");
    // The actor moved to a new mediator; the cached document still names the old one.
    inner.put(doc(
        did.as_str(),
        vec![svc(
            did.as_str(),
            "dm",
            "DIDCommMessaging",
            "https://new.example",
            &[],
        )],
    ));
    let mut tried = Vec::new();
    let delivered = cache
        .deliver(&did, |res| {
            let uri = res.document.service[0].endpoints[0].uri.clone();
            tried.push(uri.clone());
            if uri == "https://old.example" {
                Err(AttemptError::Stale("endpoint gone"))
            } else {
                Ok(uri)
            }
        })
        .expect("delivered after re-resolution");
    assert_eq!(delivered, "https://new.example");
    assert_eq!(tried, vec!["https://old.example", "https://new.example"]);
    assert_eq!(inner.calls(), 2);

    // A second stale failure is reported after the single retry.
    let mut attempts = 0;
    let err = cache
        .deliver(&did, |_| -> Result<(), _> {
            attempts += 1;
            Err(AttemptError::Stale("still gone"))
        })
        .expect_err("fails after one retry");
    assert_eq!(err, DeliverError::Attempt("still gone"));
    assert_eq!(attempts, 2);
}
