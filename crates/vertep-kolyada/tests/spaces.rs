//! Instances: genesis, joins, role-derived authority, history and sub-protocols.
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;

use common::{AUDITOR, BUYER, SELLER, bindings, cast, open, pending, quote};
use serde_json::json;
use vertep_kolyada::testkit::SoftSigner;
use vertep_kolyada::{
    Body, Event, HistoryEntry, Instance, Message, Reason, SendError, Status, Verdict,
};

// @tg: verifies:: [[openspec:protocol-spaces#Protocol Instance#Genesis creates an instance]] {level: unit}
#[test]
fn genesis_creates_an_instance() {
    let s = pending();
    assert_eq!(s.buyer.id(), s.genesis.hash());
    assert_eq!(s.seller.id(), s.buyer.id());
    assert_eq!(s.seller.bindings(), &bindings());
    assert_eq!(s.seller.local_role(), "seller");
    assert_eq!(s.buyer.status(), Status::Pending);
}

// @tg: verifies:: [[openspec:protocol-spaces#Protocol Instance#Unsigned genesis refused]] {level: unit}
#[test]
fn unsigned_genesis_refused() {
    let s = pending();
    let mut forged = s.genesis.clone();
    forged.signature = "AAAA".into();
    let err = Instance::from_genesis(quote(), &forged, SELLER, &s.cast.verifier, None).unwrap_err();
    assert_eq!(err, Reason::Signature);
    // A genesis signed by a DID that is not the initiator's bound DID is also refused.
    let thief = SoftSigner::from_seed("did:example:thief", [9; 32]);
    let mut verifier = cast().verifier;
    verifier.add(&thief);
    let body = Body {
        sender: "did:example:thief".into(),
        ..s.genesis.body.clone()
    };
    let m = Message::sign(body, &thief).unwrap();
    assert!(matches!(
        Instance::from_genesis(quote(), &m, SELLER, &verifier, None),
        Err(Reason::BadGenesis(_))
    ));
}

// @tg: verifies:: [[openspec:protocol-spaces#Joining#Instance opens]] {level: unit}
#[test]
fn instance_opens() {
    let s = open();
    for i in [&s.buyer, &s.seller, &s.auditor] {
        assert_eq!(i.status(), Status::Open);
    }
}

// @tg: verifies:: [[openspec:protocol-spaces#Joining#Decline ends the instance]] {level: unit}
#[test]
fn decline_ends_the_instance() {
    let mut s = pending();
    let d = s.auditor.decline(&s.cast.auditor).unwrap();
    s.buyer.receive(&d, &s.cast.verifier, 0);
    assert_eq!(s.buyer.status(), Status::Declined);
    let j = s.seller.join(&s.cast.seller).unwrap();
    s.buyer.receive(&j, &s.cast.verifier, 0);
    assert_eq!(s.buyer.status(), Status::Declined, "it never opens");
}

// @tg: verifies:: [[openspec:protocol-spaces#Joining#No join, no standing]] {level: unit}
#[test]
fn no_join_no_standing() {
    let mut s = pending();
    // Only the seller has joined; the auditor has not.
    let j = s.seller.join(&s.cast.seller).unwrap();
    s.buyer.receive(&j, &s.cast.verifier, 0);
    assert_eq!(s.buyer.status(), Status::Pending);
    let err = s.buyer.send(
        &s.cast.buyer,
        "quote.request",
        json!({ "item": "a", "qty": 1 }),
        &["seller"],
    );
    assert!(matches!(err, Err(SendError::Refused(Reason::NotOpen))));
}

// @tg: verifies:: [[openspec:protocol-spaces#Role-Derived Authority#Role grants exactly the projection]] {level: unit}
#[test]
fn role_grants_exactly_the_projection() {
    let mut s = open();
    let m = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    assert_eq!(s.seller.check(&m, &s.cast.verifier), Verdict::Accepted);
    // The same sender, now outside what the projection allows, is refused: no grant is consulted.
    let m2 = s.buyer.send(
        &s.cast.buyer,
        "quote.request",
        json!({ "item": "a", "qty": 1 }),
        &["seller"],
    );
    assert!(m2.is_err());
}

// @tg: verifies:: [[openspec:protocol-spaces#Role-Derived Authority#Grant does not widen]] {level: unit}
#[test]
fn grant_does_not_widen() {
    // The crate has no grant to consult: the verdict is a function of the instance and the message
    // only. A message the projection does not allow stays refused whatever else the receiver holds.
    let mut s = open();
    let body = Body {
        protocol: s.buyer.protocol().id().as_str().into(),
        instance: Some(s.buyer.id().into()),
        sender: SELLER.into(),
        role: "seller".into(),
        seq: 7,
        deps: vec![s.genesis.hash()],
        recipients: vec!["buyer".into()],
        kind: "receipt".into(),
        payload: json!({}),
    };
    let m = Message::sign(body, &s.cast.seller).unwrap();
    assert_eq!(
        s.buyer.check(&m, &s.cast.verifier),
        Verdict::Refused(Reason::NotEnabledInState)
    );
    let _ = &mut s;
}

// @tg: verifies:: [[openspec:protocol-spaces#Pairwise Sealed Fan-Out#Only the recipient can read]] {level: unit}
#[test]
fn only_the_recipient_can_read() {
    // Sealing belongs to the transport. What the crate decides is who is entitled to each message:
    // the history a role may read holds a message only when the role sent it or it was sealed to it.
    let mut s = open();
    let m = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    let readable = |role: &str| {
        s.buyer
            .history_for(role)
            .into_iter()
            .filter_map(|e| {
                if let HistoryEntry::Full(f) = e {
                    Some(f.hash())
                } else {
                    None
                }
            })
            .any(|h| h == m.hash())
    };
    assert!(readable("seller"));
    assert!(readable("buyer"));
    assert!(!readable("auditor"));
}

// @tg: verifies:: [[openspec:protocol-spaces#Hash-Linked History#Divergence found]] {level: unit}
#[test]
fn divergence_found() {
    let mut s = open();
    let m = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    // The seller has not received the request yet; it compares heads with the buyer.
    assert_eq!(s.seller.missing_from(&s.buyer.heads()), [m.hash()]);
    s.seller.receive(&m, &s.cast.verifier, 0);
    assert!(s.seller.missing_from(&s.buyer.heads()).is_empty());
}

// @tg: verifies:: [[openspec:protocol-spaces#Hash-Linked History#Hidden message keeps the chain]] {level: unit}
#[test]
fn hidden_message_keeps_the_chain() {
    let mut s = open();
    let m1 = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    s.seller.receive(&m1, &s.cast.verifier, 0);
    let m2 = s
        .seller
        .send(
            &s.cast.seller,
            "quote",
            json!({ "price": 1.5, "currency": "USD" }),
            &["buyer", "auditor"],
        )
        .unwrap();
    s.buyer.receive(&m2, &s.cast.verifier, 0);
    // The seller forwards the history to the auditor, who may not read the request.
    let history = s.seller.history_for("auditor");
    let hidden = history
        .iter()
        .filter(|e| matches!(e, HistoryEntry::Hidden(_)))
        .count();
    assert!(hidden >= 1, "the request is hash only");
    assert!(
        history
            .iter()
            .any(|e| matches!(e, HistoryEntry::Hidden(h) if *h == m1.hash()))
    );
    // A late auditor rebuilds the instance from it; the chain still verifies end to end.
    let rebuilt = Instance::from_history(quote(), &history, AUDITOR, &s.cast.verifier).unwrap();
    assert_eq!(rebuilt.status(), Status::Open);
    assert!(rebuilt.has_applied(&m1.hash()) && rebuilt.has_applied(&m2.hash()));
    assert!(rebuilt.knowledge().contains("Decided"));
    // A doctored history is refused at the doctored entry.
    let mut doctored = history.clone();
    for e in &mut doctored {
        if let HistoryEntry::Full(m) = e
            && m.body.kind == "quote"
        {
            m.body.payload = json!({ "price": 0.01, "currency": "USD" });
        }
    }
    assert!(Instance::from_history(quote(), &doctored, AUDITOR, &s.cast.verifier).is_err());
}

// @tg: verifies:: [[openspec:protocol-spaces#Hash-Linked History#Hidden message keeps the chain]] {level: unit}
#[test]
fn held_message_waits_for_a_hidden_hash() {
    let mut s = open();
    let m1 = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    s.seller.receive(&m1, &s.cast.verifier, 0);
    let m2 = s
        .seller
        .send(
            &s.cast.seller,
            "quote",
            json!({ "price": 1.5, "currency": "USD" }),
            &["buyer", "auditor"],
        )
        .unwrap();
    // The auditor gets the quote first; it depends on a request the auditor cannot read.
    assert!(matches!(
        s.auditor.receive(&m2, &s.cast.verifier, 0).as_slice(),
        [Event::Held(_)]
    ));
    let events = s.auditor.observe_hidden(&m1.hash(), &s.cast.verifier);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Applied(h) if *h == m2.hash()))
    );
}

fn payment() -> std::sync::Arc<vertep_kolyada::Protocol> {
    std::sync::Arc::new(common::parse(&json!({
        "@context": "https://w3id.org/vertep/kolyada/1.0",
        "type": "Protocol", "version": "1.0",
        "roles": ["payer", "payee"],
        "messages": [{ "id": "pay" }, { "id": "paid" }],
        "initial": "Open", "final": ["Settled"],
        "transitions": [
            { "from": "Open", "sender": "payer", "receivers": ["payee"], "message": "pay", "to": "Paid" },
            { "from": "Paid", "sender": "payee", "receivers": ["payer"], "message": "paid", "to": "Settled" }
        ]
    })))
}

/// The buyer and seller are in the quote space and the request has reached both. The buyer
/// opens a payment instance under that request, with the buyer paying and the seller receiving.
fn parent_and_child() -> (common::Space, Message, Instance) {
    let mut s = open();
    let m1 = s
        .buyer
        .send(
            &s.cast.buyer,
            "quote.request",
            json!({ "item": "a", "qty": 1 }),
            &["seller"],
        )
        .unwrap();
    s.seller.receive(&m1, &s.cast.verifier, 0);
    let child_bindings = BTreeMap::from([
        ("payer".to_owned(), BUYER.to_owned()),
        ("payee".to_owned(), SELLER.to_owned()),
    ]);
    let (genesis, child) = Instance::create(
        payment(),
        &s.cast.buyer,
        "payer",
        &child_bindings,
        Some((&s.buyer, &m1.hash())),
    )
    .unwrap();
    (s, genesis, child)
}

// @tg: verifies:: [[openspec:protocol-spaces#Sub-Protocol Invocation#Child completes the parent step]] {level: unit}
#[test]
fn child_completes_the_parent_step() {
    let (s, genesis, mut payer) = parent_and_child();
    assert_eq!(payer.parent(), genesis.body.payload["parent"].as_str());
    assert_ne!(payer.id(), s.buyer.id(), "the child is its own instance");
    let mut payee = Instance::from_genesis(
        payment(),
        &genesis,
        SELLER,
        &s.cast.verifier,
        Some(&s.seller),
    )
    .unwrap();
    let join = payee.join(&s.cast.seller).unwrap();
    payer.receive(&join, &s.cast.verifier, 0);
    assert_eq!(payer.status(), Status::Open);
    let pay = payer
        .send(&s.cast.buyer, "pay", json!({}), &["payee"])
        .unwrap();
    payee.receive(&pay, &s.cast.verifier, 0);
    let paid = payee
        .send(&s.cast.seller, "paid", json!({}), &["payer"])
        .unwrap();
    payer.receive(&paid, &s.cast.verifier, 0);
    assert!(payer.is_complete());
    // The completion message, signed under the child protocol, is what the parent step waits for.
    let done = payer
        .send(
            &s.cast.buyer,
            vertep_kolyada::control::COMPLETE,
            json!({}),
            &["payee"],
        )
        .unwrap();
    assert_eq!(done.body.protocol, payment().id().as_str());
    payee.receive(&done, &s.cast.verifier, 0);
    assert_eq!(payer.status(), Status::Completed);
    assert_eq!(payee.status(), Status::Completed);
}

// @tg: verifies:: [[openspec:protocol-spaces#Sub-Protocol Invocation#Bindings come from the parent]] {level: unit}
#[test]
fn bindings_come_from_the_parent() {
    let (s, genesis, _) = parent_and_child();
    let parent = genesis.body.payload["parent"].as_str().unwrap().to_owned();
    let outsider = BTreeMap::from([
        ("payer".to_owned(), BUYER.to_owned()),
        ("payee".to_owned(), "did:example:stranger".to_owned()),
    ]);
    let err = Instance::create(
        payment(),
        &s.cast.buyer,
        "payer",
        &outsider,
        Some((&s.buyer, &parent)),
    )
    .unwrap_err();
    assert!(matches!(err, SendError::Refused(Reason::BadGenesis(_))));
}
