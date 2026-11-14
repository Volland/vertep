//! The signed envelope, the conformance check, causal delivery and violation proofs.
#![allow(missing_docs)]

mod common;

use common::{BUYER, cast, open, quote};
use serde_json::json;
use vertep_kolyada::testkit::SoftSigner;
use vertep_kolyada::{Body, Equivocation, Event, Message, Reason, Status, Verdict, Violation};

fn request() -> serde_json::Value {
    json!({ "item": "widget", "qty": 3 })
}

// @tg: verifies:: [[openspec:protocol-conformance#Signed Message Envelope#Signature verifies for a third party]] {level: unit}
#[test]
fn signature_verifies_for_a_third_party() {
    let mut s = open();
    let m = s
        .buyer
        .send(&s.cast.buyer, "quote.request", request(), &["seller"])
        .unwrap();
    // The auditor was not a recipient, but holding the forwarded plaintext it can verify alone.
    assert!(m.verify(&s.cast.verifier).is_ok());
    let copy = Message::from_json(&m.to_json()).unwrap();
    assert_eq!(copy.hash(), m.hash());
    assert!(copy.verify(&s.cast.verifier).is_ok());
}

// @tg: verifies:: [[openspec:protocol-conformance#Signed Message Envelope#Tampered body refused]] {level: unit}
#[test]
fn tampered_body_refused() {
    let mut s = open();
    let mut m = s
        .buyer
        .send(&s.cast.buyer, "quote.request", request(), &["seller"])
        .unwrap();
    m.body.payload = json!({ "item": "widget", "qty": 300 });
    assert!(m.verify(&s.cast.verifier).is_err());
    assert_eq!(
        s.seller.check(&m, &s.cast.verifier),
        Verdict::Refused(Reason::Signature)
    );
}

// @tg: verifies:: [[openspec:protocol-conformance#Conformance Check#Message out of state refused]] {level: unit}
// @tg: verifies:: [[openspec:protocol-spaces#Chart Role Binding#Outbound send refused locally]] {level: unit}
#[test]
fn message_out_of_state_refused() {
    let mut s = open();
    // The seller's own projection refuses to send the quote before any request.
    let err = s.seller.send(
        &s.cast.seller,
        "quote",
        json!({ "price": 9.5, "currency": "EUR" }),
        &["buyer", "auditor"],
    );
    assert!(matches!(
        err,
        Err(vertep_kolyada::SendError::Refused(
            Reason::NotEnabledInState
        ))
    ));
    // A signed one made by hand is refused by the receiver with the same reason.
    let genesis_hash = s.genesis.hash();
    let body = Body {
        protocol: s.buyer.protocol().id().as_str().into(),
        instance: Some(s.buyer.id().into()),
        sender: common::SELLER.into(),
        role: "seller".into(),
        seq: 10,
        deps: vec![genesis_hash],
        recipients: vec!["buyer".into()],
        kind: "quote".into(),
        payload: json!({ "price": 9.5, "currency": "EUR" }),
    };
    let forged = Message::sign(body, &s.cast.seller).unwrap();
    assert_eq!(
        s.buyer.check(&forged, &s.cast.verifier),
        Verdict::Refused(Reason::NotEnabledInState)
    );
}

// @tg: verifies:: [[openspec:protocol-conformance#Conformance Check#Wrong role refused]] {level: unit}
#[test]
fn wrong_role_refused() {
    let mut s = open();
    let m = s
        .buyer
        .send(&s.cast.buyer, "quote.request", request(), &["seller"])
        .unwrap();
    s.seller.receive(&m, &s.cast.verifier, 0);
    // In `Quoting` only the seller may send `quote`; the buyer sends it.
    let body = Body {
        protocol: s.buyer.protocol().id().as_str().into(),
        instance: Some(s.buyer.id().into()),
        sender: BUYER.into(),
        role: "buyer".into(),
        seq: 9,
        deps: vec![m.hash()],
        recipients: vec!["seller".into()],
        kind: "quote".into(),
        payload: json!({ "price": 1, "currency": "EUR" }),
    };
    let wrong = Message::sign(body, &s.cast.buyer).unwrap();
    assert_eq!(
        s.seller.check(&wrong, &s.cast.verifier),
        Verdict::Refused(Reason::RoleNotPermitted)
    );
    // And an unbound DID cannot take a role at all.
    let thief = SoftSigner::from_seed("did:example:thief", [9; 32]);
    let body = Body {
        sender: "did:example:thief".into(),
        ..wrong.body.clone()
    };
    let stolen = Message::sign(body, &thief).unwrap();
    assert!(matches!(
        s.seller.check(&stolen, &s.cast.verifier),
        Verdict::Refused(_)
    ));
}

// @tg: verifies:: [[openspec:protocol-conformance#Conformance Check#Payload mismatch refused]] {level: unit}
#[test]
fn payload_mismatch_refused() {
    let mut s = open();
    let err = s.buyer.send(
        &s.cast.buyer,
        "quote.request",
        json!({ "item": "widget", "qty": "three" }),
        &["seller"],
    );
    match err {
        Err(vertep_kolyada::SendError::Refused(r)) => assert_eq!(r.as_str(), "payload shape"),
        other => panic!("{other:?}"),
    }
}

/// The buyer's request, the seller's quote and the buyer's order, as the seller and buyer see them.
fn conversation() -> (common::Space, Vec<Message>) {
    let mut s = open();
    let m1 = s
        .buyer
        .send(&s.cast.buyer, "quote.request", request(), &["seller"])
        .unwrap();
    s.seller.receive(&m1, &s.cast.verifier, 0);
    let m2 = s
        .seller
        .send(
            &s.cast.seller,
            "quote",
            json!({ "price": 9.5, "currency": "EUR" }),
            &["buyer", "auditor"],
        )
        .unwrap();
    s.buyer.receive(&m2, &s.cast.verifier, 0);
    let m3 = s
        .buyer
        .send(&s.cast.buyer, "order", json!({}), &["seller"])
        .unwrap();
    (s, vec![m1, m2, m3])
}

// @tg: verifies:: [[openspec:protocol-conformance#Causal Delivery#Reordered messages applied in order]] {level: unit}
#[test]
fn reordered_messages_applied_in_order() {
    let (s, m) = conversation();
    // A view that gets the conversation backwards: order, quote, then the request.
    let mut late = open_again(&s);
    assert!(matches!(
        late.seller.receive(&m[2], &s.cast.verifier, 0).as_slice(),
        [Event::Held(_)]
    ));
    assert!(matches!(
        late.seller.receive(&m[1], &s.cast.verifier, 1).as_slice(),
        [Event::Held(_)]
    ));
    let events = late.seller.receive(&m[0], &s.cast.verifier, 2);
    let applied: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let Event::Applied(h) = e {
                Some(h.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(applied, [m[0].hash(), m[1].hash(), m[2].hash()]);
    assert_eq!(late.seller.held_len(), 0);
    assert!(late.seller.knowledge().contains("Ordered"));
}

/// A new space with the same genesis, joins and keys, so messages of `s` can be replayed in it.
fn open_again(s: &common::Space) -> common::Space {
    let cast = cast();
    let p = quote();
    let seller = vertep_kolyada::Instance::from_genesis(
        p.clone(),
        &s.genesis,
        common::SELLER,
        &cast.verifier,
        None,
    )
    .unwrap();
    let auditor = vertep_kolyada::Instance::from_genesis(
        p.clone(),
        &s.genesis,
        common::AUDITOR,
        &cast.verifier,
        None,
    )
    .unwrap();
    let buyer =
        vertep_kolyada::Instance::from_genesis(p, &s.genesis, BUYER, &cast.verifier, None).unwrap();
    let mut fresh = common::Space {
        cast,
        genesis: s.genesis.clone(),
        buyer,
        seller,
        auditor,
    };
    // Replay the joins: the seller and auditor sent them in `s`, so rebuild from the history.
    for entry in s.buyer.history_for("buyer") {
        if let vertep_kolyada::HistoryEntry::Full(m) = entry
            && m.body.kind.starts_with("kolyada:")
            && m.body.kind != "kolyada:genesis"
        {
            fresh.seller.receive(&m, &fresh.cast.verifier, 0);
            fresh.buyer.receive(&m, &fresh.cast.verifier, 0);
            fresh.auditor.receive(&m, &fresh.cast.verifier, 0);
        }
    }
    fresh
}

// @tg: verifies:: [[openspec:protocol-conformance#Causal Delivery#Timeout reports a stall]] {level: unit}
#[test]
fn timeout_reports_a_stall() {
    let (mut s, m) = conversation();
    let mut late = open_again(&s);
    late.seller.receive(&m[2], &s.cast.verifier, 100);
    assert_eq!(late.seller.held_len(), 1);
    // The protocol declares a timeout of 1000 ms.
    assert!(late.seller.expire(1000).is_empty());
    let stalled = late.seller.expire(1100);
    assert!(matches!(stalled.as_slice(), [Event::Stalled { missing, .. }] if !missing.is_empty()));
    assert_eq!(late.seller.held_len(), 0);
    let _ = &mut s;
}

// @tg: verifies:: [[openspec:protocol-conformance#Causal Delivery#Buffer is bounded]] {level: unit}
#[test]
fn buffer_is_bounded() {
    let (s, _) = conversation();
    let mut late = open_again(&s);
    // The protocol declares a buffer of 4: five messages with unknown dependencies evict the oldest.
    let mut refused = 0;
    for i in 0..5u64 {
        let body = Body {
            protocol: late.seller.protocol().id().as_str().into(),
            instance: Some(late.seller.id().into()),
            sender: BUYER.into(),
            role: "buyer".into(),
            seq: 50 + i,
            deps: vec![format!("{i:064x}")],
            recipients: vec!["seller".into()],
            kind: "quote.request".into(),
            payload: request(),
        };
        let m = Message::sign(body, &s.cast.buyer).unwrap();
        for e in late.seller.receive(&m, &s.cast.verifier, i) {
            if matches!(
                e,
                Event::Refused {
                    reason: Reason::BufferFull,
                    ..
                }
            ) {
                refused += 1;
            }
        }
    }
    assert_eq!(refused, 1);
    assert_eq!(late.seller.held_len(), 4);
}

// @tg: verifies:: [[openspec:protocol-conformance#Violation Records#Violation proof verifies]] {level: unit}
#[test]
fn violation_proof_verifies() {
    let (mut s, m) = conversation();
    let mut tampered = m[0].clone();
    tampered.body.payload = json!({ "item": "x", "qty": 1 });
    let mut recorded = None;
    for e in s.seller.receive(&tampered, &s.cast.verifier, 0) {
        if let Event::Refused {
            reason, message, ..
        } = e
        {
            recorded = Some(Violation::new(&reason, *message));
        }
    }
    let v = recorded.expect("refused");
    assert_eq!(v.reason, "signature");
    // Another participant re-runs the check on its own state and agrees.
    assert!(v.verify(&s.auditor, &s.cast.verifier));
    // A violation of a message that is in fact valid does not verify.
    let bogus = Violation {
        reason: "signature".into(),
        message: m[1].clone(),
    };
    assert!(!bogus.verify(&s.auditor, &s.cast.verifier));
}

// @tg: verifies:: [[openspec:protocol-conformance#Violation Records#Equivocation proof]] {level: unit}
#[test]
fn equivocation_proof() {
    let (mut s, m) = conversation();
    // The buyer signs a second, different message with the sequence number of its request.
    let body = Body {
        payload: json!({ "item": "gadget", "qty": 1 }),
        ..m[0].body.clone()
    };
    let second = Message::sign(body, &s.cast.buyer).unwrap();
    let events = s.seller.receive(&second, &s.cast.verifier, 0);
    let proof = events
        .iter()
        .find_map(|e| {
            if let Event::Equivocation(p) = e {
                Some(p.clone())
            } else {
                None
            }
        })
        .expect("equivocation reported");
    assert!(proof.verify(&s.cast.verifier));
    // Two messages with different sequence numbers are no proof.
    let other = Equivocation {
        a: m[0].clone(),
        b: m[2].clone(),
    };
    assert!(!other.verify(&s.cast.verifier));
    assert_eq!(s.seller.status(), Status::Open);
}
