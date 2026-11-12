//! Interop with the TSP reference implementation (`tsp_sdk`'s `SecureStore`, the OpenWallet
//! Foundation wallet): a reference sender routes a message through this mediator to a reference
//! receiver, which opens it and verifies the original sender.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::*;
use tsp_sdk::{OwnedVid, ReceivedTspMessage, RelationshipStatus, SecureStore, VerifiedVid};
use url::Url;
use vertep_federation::tsp::Vid;
use vertep_federation::{
    Did, DidDocument, DidResolver, KeyType, PublicKey, Resolution, ResolutionError,
    VerificationMethod,
};
use vertep_store::MemoryStore;
use vertep_tsp_mediator::MediatorOptions;

/// Resolves the reference wallet's VIDs from their public keys, in short and long (introduction)
/// `did:peer:4` form, and everything else through the built-in methods. Vertep's own `did:peer`
/// resolver handles numalgo 2 only.
struct Known {
    vids: Mutex<BTreeMap<String, Vid>>,
    fallback: Arc<dyn DidResolver>,
}

impl Known {
    fn add(&self, v: &Vid) {
        self.vids
            .lock()
            .unwrap()
            .insert(v.identifier().to_owned(), v.clone());
    }

    fn document(id: &str, v: &Vid) -> DidDocument {
        let id = id.to_owned();
        let mut doc = DidDocument::new(&id);
        let sig: [u8; 32] = v.verifying_key().as_ref().try_into().expect("ed25519");
        let enc: [u8; 32] = v.encryption_key().as_ref().try_into().expect("x25519");
        doc.verification_method.push(VerificationMethod::multikey(
            format!("{id}#sig"),
            &id,
            PublicKey::ed25519(sig),
        ));
        doc.verification_method.push(VerificationMethod::multikey(
            format!("{id}#enc"),
            &id,
            PublicKey::x25519(enc),
        ));
        doc.authentication.push(format!("{id}#sig"));
        doc.key_agreement.push(format!("{id}#enc"));
        doc.service.push(tsp_service(v.endpoint().as_str(), &[]));
        assert_eq!(
            doc.verification_method[0].public_key.key_type,
            KeyType::Ed25519
        );
        doc
    }
}

impl DidResolver for Known {
    fn resolve(&self, did: &Did) -> Result<Resolution, ResolutionError> {
        let asked = did.as_str();
        let vids = self.vids.lock().unwrap();
        let found = vids
            .iter()
            .find(|(short, _)| asked == short.as_str() || asked.starts_with(&format!("{short}:")));
        if let Some((_, v)) = found {
            return Ok(Resolution::of(Self::document(asked, v)));
        }
        drop(vids);
        self.fallback.resolve(did)
    }
}

// @tg: verifies:: [[openspec:tsp-mediator#TSP Routed-Mode Intermediary#Interop with the reference implementation]] {level: integration}
#[test]
fn a_reference_sender_routes_through_the_mediator_to_a_reference_receiver() {
    let w = World::new();
    let known = Arc::new(Known {
        vids: Mutex::new(BTreeMap::new()),
        fallback: w.resolver.clone(),
    });
    // The mediator resolves the reference VIDs through `known`.
    let km: Arc<dyn vertep_federation::KeyManager> =
        Arc::new(vertep_federation::InMemoryKeyManager::new());
    let identity = peer(km.as_ref(), vec![tsp_service(M1, &[])]);
    let store =
        vertep_tsp_mediator::MailboxStore::open(Arc::new(MemoryStore::new())).expect("store");
    let m = vertep_tsp_mediator::Mediator::new(
        identity,
        km,
        known.clone(),
        w.net.clone(),
        store,
        w.clock.clone(),
        MediatorOptions::default(),
    )
    .expect("mediator");
    w.net.serve(M1, m.clone());
    let m_vid = w.vid(m.did());

    // Reference sender S and receiver R, each in its own wallet.
    let s = OwnedVid::new_did_peer(Url::parse("https://s.example/tsp").unwrap());
    let r = OwnedVid::new_did_peer(Url::parse("https://r.example/tsp").unwrap());
    let (s_vid, r_vid) = (s.vid().clone(), r.vid().clone());
    let (s_id, r_id) = (s_vid.identifier().to_owned(), r_vid.identifier().to_owned());
    known.add(&s_vid);
    known.add(&r_vid);

    let sender = SecureStore::new();
    sender.add_private_vid(s, None).unwrap();
    sender.add_verified_vid(r_vid.clone(), None).unwrap();
    sender.add_verified_vid(m_vid.clone(), None).unwrap();
    sender
        .set_relation_and_status_for_vid(m_vid.identifier(), RelationshipStatus::Unrelated, &s_id)
        .unwrap();
    sender
        .set_route_for_vid(&r_id, [m_vid.identifier(), r_id.as_str()])
        .unwrap();

    let receiver = SecureStore::new();
    receiver.add_private_vid(r, None).unwrap();
    receiver.add_verified_vid(s_vid.clone(), None).unwrap();
    receiver.add_verified_vid(m_vid.clone(), None).unwrap();
    // S and R are related end to end (formed out of band here; the mediator is not involved).
    let related = || RelationshipStatus::Bidirectional {
        invite_digest: Default::default(),
        reply_digest: Default::default(),
        outstanding_nested_requests: Vec::new(),
    };
    sender
        .set_relation_and_status_for_vid(&r_id, related(), &s_id)
        .unwrap();
    receiver
        .set_relation_and_status_for_vid(&s_id, related(), &r_id)
        .unwrap();
    // R and the mediator form the neighbour relationship TSP 5.3 requires of the last hop.
    let (_, rfi) = receiver
        .make_relationship_request(&r_id, m_vid.identifier(), None)
        .expect("request");
    let mut rfa =
        vertep_federation::RelayNetwork::post(w.net.as_ref(), M1, &rfi).expect("answered");
    assert!(matches!(
        receiver.open_message(&mut rfa).expect("opens the accept"),
        ReceivedTspMessage::AcceptRelationship { .. }
    ));
    let inbox = Arc::new(Inbox::default());
    w.net.serve("https://r.example/tsp", inbox.clone());

    let (_, bytes) = sender
        .seal_message(&s_id, &r_id, b"hello through vt-mediator")
        .expect("reference seals routed");
    let resp = vertep_federation::RelayNetwork::post(w.net.as_ref(), M1, &bytes);
    assert!(resp.is_ok(), "the mediator accepts: {resp:?}");
    assert_eq!(m.metrics().delivered_direct, 1);

    let mut got = inbox.0.lock().unwrap().pop().expect("delivered to R");
    match receiver.open_message(&mut got).expect("reference opens") {
        ReceivedTspMessage::GenericMessage {
            sender, message, ..
        } => {
            assert_eq!(sender, s_id, "the original sender, verified");
            assert_eq!(message, &b"hello through vt-mediator"[..]);
        }
        other => panic!("expected a generic message, got {other:?}"),
    }
}
