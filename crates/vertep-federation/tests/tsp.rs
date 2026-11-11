//! TSP direct mode: sealing, opening and relationship setup through the key manager.
#![cfg(feature = "tsp")]

use std::sync::Arc;
use std::time::Duration;

use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::tsp::{
    Relationship, SealedTsp, TspError, TspIdentity, TspReceived, TspSession, Vid, vid_from_document,
};
use vertep_federation::vocab::TSP_PROCESSOR;
use vertep_federation::{
    ActorIdentity, CommunicationError, CommunicationFailure, Did, DidMethodChoice, DidResolver,
    IdentityConfig, InMemoryKeyManager, KeyManager, OpenedMessage, RelayEnvelope, ResolverRegistry,
    Service, ServiceEndpoint,
};

struct Actor {
    session: TspSession,
    did: String,
}

fn actor(km: &Arc<InMemoryKeyManager>, host: &str) -> Actor {
    let config = IdentityConfig::new(DidMethodChoice::Peer {
        services: vec![Service {
            id: "#tsp".into(),
            types: vec!["TSPTransport".into()],
            endpoints: vec![ServiceEndpoint::uri(format!("https://{host}/tsp"))],
        }],
    });
    let identity = ActorIdentity::provision(km.as_ref(), &config).expect("identity");
    let me = TspIdentity::new(
        &identity,
        km.clone() as Arc<dyn KeyManager>,
        format!("https://{host}/tsp").parse().expect("url"),
    )
    .expect("vid");
    Actor {
        session: TspSession::new(me, true),
        did: identity.did,
    }
}

fn resolver() -> impl Fn(&str) -> Result<Vid, TspError> {
    let registry = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    move |id: &str| {
        let did = Did::parse(id).map_err(|e| TspError::NoVid(e.to_string()))?;
        let doc = registry
            .resolve(&did)
            .map_err(|e| TspError::NoVid(e.to_string()))?
            .document;
        vid_from_document(&doc)
    }
}

fn envelope(id: &str, name: &str) -> RelayEnvelope {
    let mut e = Event::named(name, EventType::External);
    e.sendid = Some(format!("send-{id}"));
    e.data = Value::Object(vec![("item".into(), Value::String("widget".into()))]);
    RelayEnvelope::from_event(id, &e)
}

/// Delivers sealed messages to `to`, answering relationship requests per `accept`, and collects
/// the events it opens.
fn deliver_to<'a>(
    to: &'a Actor,
    accept: bool,
    inbox: &'a mut Vec<OpenedMessage>,
) -> impl FnMut(SealedTsp) -> Result<Option<Vec<u8>>, TspError> + 'a {
    let resolve = resolver();
    move |sealed: SealedTsp| {
        let mut bytes = sealed.bytes;
        match to.session.open(&mut bytes, &resolve)? {
            TspReceived::RelationshipRequested { sender, thread_id } => {
                let peer = resolve(&sender)?;
                let reply = if accept {
                    to.session.accept_relationship(&peer, thread_id)?
                } else {
                    to.session.refuse_relationship(&peer, thread_id)?
                };
                Ok(Some(reply.bytes))
            }
            TspReceived::Event(m) => {
                inbox.push(*m);
                Ok(None)
            }
            other => Err(TspError::Unexpected(format!("{other:?}"))),
        }
    }
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Direct Transport#Direct exchange]] {level: integration}
#[test]
fn direct_exchange_arrives_with_verified_origin() {
    let km = Arc::new(InMemoryKeyManager::new());
    let a = actor(&km, "a.example");
    let b = actor(&km, "b.example");
    let resolve = resolver();
    let b_vid = resolve(&b.did).expect("b vid");
    let a_vid = resolve(&a.did).expect("a vid");

    let mut b_inbox = Vec::new();
    a.session
        .send_with_relationship(
            &b_vid,
            &envelope("m-1", "quote.request"),
            &resolve,
            &mut deliver_to(&b, true, &mut b_inbox),
        )
        .expect("sent");
    assert_eq!(
        a.session.relationship(&b.did),
        Some(Relationship::Established)
    );
    assert_eq!(
        b.session.relationship(&a.did),
        Some(Relationship::Established)
    );
    assert_eq!(b_inbox.len(), 1);
    let got = &b_inbox[0];
    assert_eq!(got.envelope.name, "quote.request");
    assert!(got.origin.verified);
    assert_eq!(got.origin.sender.as_deref(), Some(a.did.as_str()));
    assert_eq!(got.origin.io_processor, TSP_PROCESSOR);

    // And back, over the same relationship.
    let reply = b
        .session
        .seal_event(&a_vid, &envelope("m-2", "quote.offer"))
        .expect("sealed");
    assert_eq!(reply.endpoint.as_str(), "https://a.example/tsp");
    let mut bytes = reply.bytes;
    let TspReceived::Event(back) = a.session.open(&mut bytes, &resolve).expect("opens") else {
        panic!("expected an event");
    };
    assert_eq!(back.envelope.name, "quote.offer");
    assert_eq!(back.origin.sender.as_deref(), Some(b.did.as_str()));
}

// @tg: verifies:: [[openspec:relay-mailboxes#TSP Direct Transport#Relationship required]] {level: integration}
#[test]
fn relationship_is_set_up_first_or_the_send_fails_when_refused() {
    let km = Arc::new(InMemoryKeyManager::new());
    let a = actor(&km, "a.example");
    let b = actor(&km, "b.example");
    let resolve = resolver();
    let b_vid = resolve(&b.did).expect("b vid");

    // No relationship: the event itself is not sealed.
    assert!(matches!(
        a.session
            .seal_event(&b_vid, &envelope("m-0", "quote.request")),
        Err(TspError::RelationshipRequired(_))
    ));

    // The recipient refuses: the send fails with error.communication and nothing is delivered.
    let mut inbox = Vec::new();
    let err = a
        .session
        .send_with_relationship(
            &b_vid,
            &envelope("m-1", "quote.request"),
            &resolve,
            &mut deliver_to(&b, false, &mut inbox),
        )
        .expect_err("refused");
    assert!(matches!(err, TspError::RelationshipRefused(_)));
    assert!(inbox.is_empty());
    let comm: CommunicationError = err.into();
    assert!(matches!(comm.failure, CommunicationFailure::Refused(_)));
    assert_eq!(comm.for_send("s-1").to_event().name, "error.communication");
    assert_eq!(a.session.relationship(&b.did), None);
}

#[test]
fn receiver_refuses_events_outside_a_relationship_and_tampered_messages() {
    let km = Arc::new(InMemoryKeyManager::new());
    let a = actor(&km, "a.example");
    let b = actor(&km, "b.example");
    let resolve = resolver();
    let b_vid = resolve(&b.did).expect("b vid");
    let relaxed = TspSession::new(a.session.identity().clone(), false);
    let mut sealed = relaxed
        .seal_event(&b_vid, &envelope("m-1", "quote.request"))
        .expect("sealed without relationship");
    let mut copy = sealed.bytes.clone();
    assert!(matches!(
        b.session.open(&mut copy, &resolve),
        Err(TspError::RelationshipRequired(_))
    ));
    let last = sealed.bytes.len() - 10;
    sealed.bytes[last] ^= 0x01;
    assert!(matches!(
        b.session.open(&mut sealed.bytes, &resolve),
        Err(TspError::Open(_))
    ));
    let km_secret_free = format!("{:?}", a.session);
    assert!(!km_secret_free.contains("secret"));
}
