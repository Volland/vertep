//! DID-scoped grants: inbound, outbound, credential-based, and in the graph.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use oxigraph::sparql::{QueryResults, SparqlEvaluator};
use oxigraph::store::Store;
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::credential::issue_grant_credential;
use vertep_federation::keys::seal_to;
use vertep_federation::ontology::{project_grants, project_inbound, validate};
use vertep_federation::origin::Untyped;
use vertep_federation::{
    ActorIdentity, CommunicationFailure, CredentialError, DidDirectory, DidMethodChoice, Grant,
    GrantSet, IdentityConfig, InMemoryKeyManager, InboundPolicy, InboundRecord, KeyManager,
    OpenedMessage, Refusal, RelayEnvelope, ResolverRegistry, Timestamp, VerifiedOrigin, admit,
    verify_grant_credential,
};

const AGENT: &str = "did:webvh:QmScid:example.org:agents:billing";
const SENDER_B: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

fn opened(sender: &str, name: &str) -> OpenedMessage {
    let mut e = Event::named(name, EventType::External);
    e.data = Value::Object(vec![("id".into(), Value::String("o-1".into()))]);
    OpenedMessage {
        envelope: RelayEnvelope::from_event(format!("m-{name}"), &e),
        origin: VerifiedOrigin::verified(sender, format!("{sender}#k"), "urn:test:processor"),
    }
}

fn policy(grants: GrantSet) -> InboundPolicy {
    InboundPolicy {
        actor: AGENT.into(),
        anonymous: Default::default(),
        grants,
    }
}

fn order_grants() -> GrantSet {
    let mut g = GrantSet::new();
    g.add(Grant::new(SENDER_B, AGENT, ["order.*"]));
    g
}

// @tg: verifies:: [[openspec:did-identity#DID-Scoped Capability Grants#Granted sender]] {level: integration}
#[test]
fn granted_sender_is_delivered() {
    let admitted = admit(
        &policy(order_grants()),
        &opened(SENDER_B, "order.placed"),
        &Untyped,
        Timestamp::from_secs(0),
    )
    .expect("delivered");
    assert_eq!(admitted.event.name, "order.placed");
    assert_eq!(admitted.event.origin.as_deref(), Some(SENDER_B));
}

// @tg: verifies:: [[openspec:did-identity#DID-Scoped Capability Grants#Event outside the grant]] {level: integration}
#[test]
fn event_outside_the_grant_is_refused_and_recorded() {
    let msg = opened(SENDER_B, "admin.reset");
    let refusal = admit(
        &policy(order_grants()),
        &msg,
        &Untyped,
        Timestamp::from_secs(0),
    )
    .expect_err("not granted");
    assert!(matches!(refusal, Refusal::NotGranted(_)));
    assert!(
        refusal.notifies_sender(),
        "the sender gets error.communication"
    );
    let record = InboundRecord::refused(AGENT, Some(&msg), refusal, Timestamp::from_secs(0));
    let g = project_inbound(&record).expect("projects");
    assert!(validate(&g).conforms());
    assert!(g.to_string().contains("admin.reset"));
}

// @tg: verifies:: [[openspec:did-identity#DID-Scoped Capability Grants#Expired grant]] {level: integration}
#[test]
fn expired_grant_is_refused_as_ungranted() {
    let mut grants = GrantSet::new();
    grants.add(Grant::new(SENDER_B, AGENT, ["order.*"]).expiring(Timestamp::from_secs(100)));
    let p = policy(grants);
    admit(
        &p,
        &opened(SENDER_B, "order.placed"),
        &Untyped,
        Timestamp::from_secs(99),
    )
    .expect("still valid");
    let refusal = admit(
        &p,
        &opened(SENDER_B, "order.placed"),
        &Untyped,
        Timestamp::from_secs(100),
    )
    .expect_err("expired");
    assert!(matches!(&refusal, Refusal::NotGranted(m) if m.contains("expired")));
}

// @tg: verifies:: [[openspec:did-identity#Outbound Grant Enforcement#Ungranted outbound send]] {level: integration}
#[test]
fn ungranted_outbound_send_raises_error_and_produces_no_envelope() {
    let llm_actor = "vt:actor/llm-1";
    let external = "did:web:partner.example";
    let grants = GrantSet::new();
    let sealed = Cell::new(false);
    let err = grants
        .send_if_granted(
            llm_actor,
            external,
            "quote.request",
            Some("s-3"),
            Timestamp::from_secs(0),
            || {
                sealed.set(true);
                Ok(Vec::<u8>::new())
            },
        )
        .expect_err("refused");
    assert!(!sealed.get(), "no envelope was produced");
    assert!(matches!(err.failure, CommunicationFailure::NotGranted(_)));
    let ev = err.to_event();
    assert_eq!(ev.name, "error.communication");
    assert_eq!(ev.sendid.as_deref(), Some("s-3"));
}

// @tg: verifies:: [[openspec:did-identity#Outbound Grant Enforcement#Granted outbound send]] {level: integration}
#[test]
fn granted_outbound_send_is_encrypted_and_handed_to_the_transport() {
    let km = InMemoryKeyManager::new();
    let partner = ActorIdentity::provision(
        &km,
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![vertep_federation::Service {
                id: "#tsp".into(),
                types: vec!["TSPTransport".into()],
                endpoints: vec![vertep_federation::ServiceEndpoint::uri(
                    "https://partner.example/tsp",
                )],
            }],
        }),
    )
    .expect("partner");
    let dir = DidDirectory::new(Arc::new(ResolverRegistry::with_builtin(
        None,
        Duration::from_secs(1),
    )));
    let llm_actor = "vt:actor/llm-1";
    let mut grants = GrantSet::new();
    // The invoking state granted the LLM actor this one external target.
    grants.add(Grant::new(llm_actor, partner.did.as_str(), ["quote.*"]));
    let mut transport: Vec<(String, Vec<u8>)> = Vec::new();
    let env = RelayEnvelope::from_event("m-1", &Event::named("quote.request", EventType::External));
    grants
        .send_if_granted(
            llm_actor,
            &partner.did,
            "quote.request",
            Some("s-4"),
            Timestamp::from_secs(0),
            || {
                let resolved = dir.resolve(&partner.did)?;
                let key = resolved.encryption_keys()[0].public_key;
                let sealed = seal_to(&key, b"vertep-relay", b"", &env.to_bytes())
                    .map_err(|e| CommunicationFailure::SealFailed(e.to_string()))?;
                transport.push((resolved.routes[0].endpoint.uri.clone(), sealed.ciphertext));
                Ok(())
            },
        )
        .expect("granted");
    assert_eq!(transport.len(), 1);
    assert_eq!(transport[0].0, "https://partner.example/tsp");
    assert!(
        !String::from_utf8_lossy(&transport[0].1).contains("quote.request"),
        "the payload is encrypted"
    );
}

fn issuer() -> (InMemoryKeyManager, ActorIdentity) {
    let km = InMemoryKeyManager::new();
    let id =
        ActorIdentity::provision(&km, &IdentityConfig::new(DidMethodChoice::Key)).expect("issuer");
    (km, id)
}

// @tg: verifies:: [[openspec:did-identity#Credential-Based Grants#Valid credential grant]] {level: integration}
#[test]
fn valid_credential_grant_from_a_trusted_issuer_is_honoured() {
    let (km, iss) = issuer();
    let sender = "did:key:z6MkiTBz1ymuepAQ4HEHYSF1H8quG5GLVVQR3djdX3mDooWp";
    let grant = Grant::new(sender, AGENT, ["quote.*"]).expiring(Timestamp::from_secs(10_000));
    let vc = issue_grant_credential(&km, &iss, &grant, Timestamp::from_secs(0)).expect("issued");
    let registry = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    let verified = verify_grant_credential(
        &vc,
        AGENT,
        std::slice::from_ref(&iss.did),
        &registry,
        Timestamp::from_secs(5),
    )
    .expect("verifies");
    assert_eq!(verified.issuer.as_deref(), Some(iss.did.as_str()));
    let mut grants = GrantSet::new();
    grants.add(verified);
    admit(
        &policy(grants),
        &opened(sender, "quote.request"),
        &Untyped,
        Timestamp::from_secs(5),
    )
    .expect("delivered under the credential grant");

    // A tampered credential, an expired one and one for another target are refused.
    let mut tampered = vc.clone();
    tampered["credentialSubject"]["events"] = serde_json::json!(["*"]);
    assert!(matches!(
        verify_grant_credential(
            &tampered,
            AGENT,
            std::slice::from_ref(&iss.did),
            &registry,
            Timestamp::from_secs(5)
        ),
        Err(CredentialError::BadProof(_))
    ));
    assert_eq!(
        verify_grant_credential(
            &vc,
            AGENT,
            std::slice::from_ref(&iss.did),
            &registry,
            Timestamp::from_secs(10_000)
        ),
        Err(CredentialError::NotValidNow)
    );
    assert!(matches!(
        verify_grant_credential(
            &vc,
            "did:example:other",
            std::slice::from_ref(&iss.did),
            &registry,
            Timestamp::from_secs(5)
        ),
        Err(CredentialError::WrongTarget(_))
    ));
}

// @tg: verifies:: [[openspec:did-identity#Credential-Based Grants#Credential from an untrusted issuer]] {level: integration}
#[test]
fn credential_from_an_untrusted_issuer_is_ignored() {
    let (km, iss) = issuer();
    let sender = "did:key:z6MkiTBz1ymuepAQ4HEHYSF1H8quG5GLVVQR3djdX3mDooWp";
    let grant = Grant::new(sender, AGENT, ["quote.*"]);
    let vc = issue_grant_credential(&km, &iss, &grant, Timestamp::from_secs(0)).expect("issued");
    let registry = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    let trusted = vec!["did:web:trusted.example".to_owned()];
    let err = verify_grant_credential(&vc, AGENT, &trusted, &registry, Timestamp::from_secs(1))
        .expect_err("untrusted");
    assert_eq!(err, CredentialError::UntrustedIssuer(iss.did.clone()));
    // The grant is ignored, so the sender is refused as ungranted.
    let refusal = admit(
        &policy(GrantSet::new()),
        &opened(sender, "quote.request"),
        &Untyped,
        Timestamp::from_secs(1),
    )
    .expect_err("refused");
    assert!(matches!(refusal, Refusal::NotGranted(_)));
    let _ = km.contains(&iss.signing.key_name);
}

// @tg: verifies:: [[openspec:did-identity#DID Documents In The Runtime Graph#Who may talk to this agent]] {level: integration}
#[test]
fn who_may_talk_to_this_agent_is_a_sparql_query() {
    let mut grants = GrantSet::new();
    grants.add(Grant::new(SENDER_B, AGENT, ["order.*", "order.cancel"]));
    grants.add(
        Grant::new("did:web:partner.example", AGENT, ["quote.request"])
            .expiring(Timestamp::from_secs(2_000_000_000)),
    );
    grants.add(Grant::new(SENDER_B, "did:web:other.example", ["ping"]));
    let g = project_grants(grants.grants(), Some(AGENT)).expect("projects");
    let report = validate(&g);
    assert!(report.conforms(), "{report}");

    let store = Store::new().expect("store");
    for t in &g {
        store
            .insert(t.in_graph(oxrdf::GraphNameRef::DefaultGraph))
            .expect("insert");
    }
    let q = format!(
        "PREFIX vt-id: <https://w3id.org/vertep/vt-id#>
         SELECT ?grantee ?event ?until WHERE {{
           ?g a vt-id:Grant ;
              vt-id:target <{AGENT}> ;
              vt-id:grantee ?grantee ;
              vt-id:event ?event .
           OPTIONAL {{ ?g vt-id:expires ?until }}
         }}"
    );
    let QueryResults::Solutions(solutions) = SparqlEvaluator::new()
        .parse_query(&q)
        .expect("parses")
        .on_store(&store)
        .execute()
        .expect("runs")
    else {
        panic!("expected solutions");
    };
    let mut rows = BTreeSet::new();
    for s in solutions {
        let s = s.expect("solution");
        rows.insert((
            s.get("grantee").expect("grantee").to_string(),
            s.get("event").expect("event").to_string(),
            s.get("until").is_some(),
        ));
    }
    let expected: BTreeSet<(String, String, bool)> = [
        (format!("<{SENDER_B}>"), "\"order.*\"".to_owned(), false),
        (
            format!("<{SENDER_B}>"),
            "\"order.cancel\"".to_owned(),
            false,
        ),
        (
            "<did:web:partner.example>".to_owned(),
            "\"quote.request\"".to_owned(),
            true,
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(rows, expected);
}

// @tg: verifies:: [[openspec:did-identity#Journaled Verification Outcome#Audit query on verified origins]] {level: integration}
#[test]
fn audit_query_lists_events_accepted_from_a_did_with_their_outcome() {
    let mut grants = GrantSet::new();
    grants.add(Grant::new(SENDER_B, AGENT, ["order.*"]));
    let p = policy(grants);
    let store = Store::new().expect("store");
    let put = |record: InboundRecord| {
        let g = project_inbound(&record).expect("projects");
        assert!(validate(&g).conforms());
        for t in &g {
            store
                .insert(t.in_graph(oxrdf::GraphNameRef::DefaultGraph))
                .expect("insert");
        }
    };
    for (i, name) in ["order.placed", "order.paid", "admin.reset"]
        .iter()
        .enumerate()
    {
        let now = Timestamp::from_secs(i as i64);
        let msg = opened(SENDER_B, name);
        match admit(&p, &msg, &Untyped, now) {
            Ok(a) => put(InboundRecord::accepted(AGENT, &a, now)),
            Err(r) => put(InboundRecord::refused(AGENT, Some(&msg), r, now)),
        }
    }
    let other = opened("did:web:other.example", "order.placed");
    let r = admit(&p, &other, &Untyped, Timestamp::from_secs(9)).expect_err("ungranted");
    put(InboundRecord::refused(
        AGENT,
        Some(&other),
        r,
        Timestamp::from_secs(9),
    ));

    let q = format!(
        "PREFIX vt: <https://w3id.org/vertep/vt#>
         PREFIX vt-id: <https://w3id.org/vertep/vt-id#>
         SELECT ?event ?verified WHERE {{
           ?m a vt-id:InboundMessage ;
              vt-id:receivedBy <{AGENT}> ;
              vt-id:verifiedOrigin <{SENDER_B}> ;
              vt-id:accepted true ;
              vt-id:originVerified ?verified ;
              vt:eventName ?event .
         }} ORDER BY ?event"
    );
    let QueryResults::Solutions(solutions) = SparqlEvaluator::new()
        .parse_query(&q)
        .expect("parses")
        .on_store(&store)
        .execute()
        .expect("runs")
    else {
        panic!("expected solutions");
    };
    let rows: Vec<(String, String)> = solutions
        .map(|s| {
            let s = s.expect("solution");
            (
                s.get("event").expect("event").to_string(),
                s.get("verified").expect("verified").to_string(),
            )
        })
        .collect();
    let t = "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_owned();
    assert_eq!(
        rows,
        vec![
            ("\"order.paid\"".to_owned(), t.clone()),
            ("\"order.placed\"".to_owned(), t),
        ]
    );
}
