//! Task 10.2: interop with the TSP reference implementation (`tsp_sdk`'s `SecureStore`, the
//! OpenWallet Foundation wallet) in direct mode, both ways, and in routed mode with the
//! reference wallet as the intermediary.
#![cfg(feature = "tsp")]

use std::sync::Arc;

use tsp_sdk::{OwnedVid, ReceivedTspMessage, RelationshipStatus, SecureStore, VerifiedVid};
use url::Url;
use vertep_core::datamodel::{Event, EventType};
use vertep_federation::tsp::{TspError, TspIdentity, TspReceived, TspSession, Vid};
use vertep_federation::{
    ActorIdentity, DidMethodChoice, IdentityConfig, InMemoryKeyManager, KeyManager, RelayEnvelope,
    Service, ServiceEndpoint,
};

fn ours(uri: &str) -> TspIdentity {
    let km: Arc<dyn KeyManager> = Arc::new(InMemoryKeyManager::new());
    let id = ActorIdentity::provision(
        km.as_ref(),
        &IdentityConfig::new(DidMethodChoice::Peer {
            services: vec![Service {
                id: "#tsp".into(),
                types: vec!["TSPTransport".into()],
                endpoints: vec![ServiceEndpoint::uri(uri)],
            }],
        }),
    )
    .expect("identity");
    TspIdentity::new(&id, km, Url::parse(uri).expect("url")).expect("vid")
}

fn public(v: &OwnedVid) -> Vid {
    v.vid().clone()
}

fn envelope() -> RelayEnvelope {
    let mut e = Event::named("quote.request", EventType::External);
    e.sendid = Some("q-1".into());
    RelayEnvelope::from_event("m-interop", &e)
}

fn resolver(vids: Vec<Vid>) -> impl Fn(&str) -> Result<Vid, TspError> {
    move |id: &str| {
        vids.iter()
            .find(|v| v.identifier() == id)
            .cloned()
            .ok_or_else(|| TspError::NoVid(id.to_owned()))
    }
}

#[test]
fn direct_mode_with_the_reference_wallet() {
    let me = ours("https://vertep.example/tsp");
    let my_vid = me.vid().clone();
    let peer = OwnedVid::new_did_peer(Url::parse("https://ref.example/tsp").expect("url"));
    let peer_vid = public(&peer);
    let wallet = SecureStore::new();
    wallet.add_private_vid(peer, None).expect("private");
    wallet
        .add_verified_vid(my_vid.clone(), None)
        .expect("verified");

    // Vertep to reference: relationship first, then the event.
    let session = TspSession::new(me, true);
    let resolve = resolver(vec![peer_vid.clone(), my_vid.clone()]);
    let mut got = Vec::new();
    session
        .send_with_relationship(&peer_vid, &envelope(), &resolve, &mut |sealed| {
            let mut bytes = sealed.bytes;
            match wallet.open_message(&mut bytes).expect("reference opens") {
                ReceivedTspMessage::RequestRelationship {
                    sender,
                    receiver,
                    thread_id,
                    ..
                } => {
                    let (_, accept) = wallet
                        .make_relationship_accept(&receiver, &sender, thread_id, None)
                        .expect("accept");
                    Ok(Some(accept))
                }
                ReceivedTspMessage::GenericMessage {
                    sender, message, ..
                } => {
                    assert_eq!(sender, my_vid.identifier());
                    got.push(message.to_vec());
                    Ok(None)
                }
                other => panic!("unexpected {other:?}"),
            }
        })
        .expect("sent");
    assert_eq!(got.len(), 1);
    let env = RelayEnvelope::from_bytes(&got[0]).expect("envelope");
    assert_eq!(env.name, "quote.request");
    assert_eq!(env.sendid.as_deref(), Some("q-1"));

    // Reference to Vertep: the reference seals, Vertep opens and verifies the sender.
    let (_, mut bytes) = wallet
        .seal_message(
            peer_vid.identifier(),
            my_vid.identifier(),
            &envelope().to_bytes(),
        )
        .expect("reference seals");
    match session.open(&mut bytes, &resolve).expect("opens") {
        TspReceived::Event(m) => {
            assert_eq!(m.origin.sender.as_deref(), Some(peer_vid.identifier()));
            assert!(m.origin.verified);
            assert_eq!(m.envelope.name, "quote.request");
        }
        other => panic!("expected an event, got {other:?}"),
    }
}

#[test]
fn routed_mode_through_the_reference_wallet_as_intermediary() {
    let sender = ours("https://s.example/tsp");
    let recipient = ours("https://r.example/tsp");
    let s_vid = sender.vid().clone();
    let r_vid = recipient.vid().clone();
    let hop = OwnedVid::new_did_peer(Url::parse("https://hop.example/tsp").expect("url"));
    let hop_vid = public(&hop);
    let hop_id = hop_vid.identifier().to_owned();
    let wallet = SecureStore::new();
    wallet.add_private_vid(hop, None).expect("private");
    wallet
        .add_verified_vid(s_vid.clone(), None)
        .expect("sender");
    wallet
        .add_verified_vid(r_vid.clone(), None)
        .expect("recipient");
    // The intermediary delivers to the recipient over its own VID.
    wallet
        .set_relation_and_status_for_vid(r_vid.identifier(), RelationshipStatus::Unrelated, &hop_id)
        .expect("drop-off relation");

    let s = TspSession::new(sender, false);
    let sealed = s
        .seal_routed(&r_vid, std::slice::from_ref(&hop_vid), &envelope())
        .expect("routed");
    let mut bytes = sealed.bytes;
    let (next, route, opaque) = match wallet.open_message(&mut bytes).expect("hop opens") {
        ReceivedTspMessage::ForwardRequest {
            next_hop,
            route,
            opaque_payload,
            ..
        } => (next_hop, route, opaque_payload),
        other => panic!("expected a forward request, got {other:?}"),
    };
    assert_eq!(next, r_vid.identifier());
    let text = String::from_utf8_lossy(&opaque);
    assert!(
        !text.contains("quote.request"),
        "the hop cannot read the event"
    );
    let route: Vec<&[u8]> = route.iter().map(|b| b.as_ref()).collect();
    let (_, mut delivered) = wallet
        .forward_routed_message(&next, route, &opaque)
        .expect("forwarded");

    let r = TspSession::new(recipient, false);
    let resolve = resolver(vec![hop_vid, s_vid.clone(), r_vid]);
    match r.open(&mut delivered, &resolve).expect("recipient opens") {
        TspReceived::Event(m) => {
            assert_eq!(m.origin.sender.as_deref(), Some(s_vid.identifier()));
            assert_eq!(m.envelope.name, "quote.request");
        }
        other => panic!("expected an event, got {other:?}"),
    }
}
