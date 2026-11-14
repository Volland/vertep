//! A three-party quote protocol and the signers that play it.
#![allow(dead_code, missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vertep_kolyada::testkit::{SoftSigner, StaticVerifier};
use vertep_kolyada::{Instance, Message, Protocol};

pub const BUYER: &str = "did:example:buyer";
pub const SELLER: &str = "did:example:seller";
pub const AUDITOR: &str = "did:example:auditor";

/// The quote protocol: buyer asks, seller quotes (the auditor sees the quote), buyer orders or
/// cancels, seller confirms. The auditor never sees `order`, `cancel` or `receipt`.
pub fn quote_doc() -> Value {
    json!({
        "@context": "https://w3id.org/vertep/kolyada/1.0",
        "type": "Protocol",
        "version": "1.0",
        "roles": ["buyer", "seller", "auditor"],
        "messages": [
            { "id": "quote.request", "payload": { "fields": [
                { "name": "item", "datatype": "string", "min": 1, "max": 1 },
                { "name": "qty", "datatype": "integer", "min": 1, "max": 1 }
            ] } },
            { "id": "quote", "payload": { "fields": [
                { "name": "price", "datatype": "number", "min": 1, "max": 1 },
                { "name": "currency", "datatype": "string", "in": ["EUR", "USD"] }
            ] } },
            { "id": "order" },
            { "id": "cancel" },
            { "id": "receipt" }
        ],
        "initial": "Idle",
        "final": ["Cancelled", "Done"],
        "transitions": [
            { "from": "Idle", "sender": "buyer", "receivers": ["seller"], "message": "quote.request", "to": "Quoting" },
            { "from": "Quoting", "sender": "seller", "receivers": ["buyer", "auditor"], "message": "quote", "to": "Decided" },
            { "from": "Decided", "sender": "buyer", "receivers": ["seller"], "message": "order", "to": "Ordered" },
            { "from": "Decided", "sender": "buyer", "receivers": ["seller"], "message": "cancel", "to": "Cancelled" },
            { "from": "Ordered", "sender": "seller", "receivers": ["buyer"], "message": "receipt", "to": "Done" }
        ],
        "limits": { "bufferSize": 4, "timeoutMs": 1000 }
    })
}

pub fn parse(doc: &Value) -> Protocol {
    Protocol::parse(doc.to_string().as_bytes()).expect("the protocol parses")
}

pub fn quote() -> Arc<Protocol> {
    Arc::new(parse(&quote_doc()))
}

pub struct Cast {
    pub buyer: SoftSigner,
    pub seller: SoftSigner,
    pub auditor: SoftSigner,
    pub verifier: StaticVerifier,
}

pub fn cast() -> Cast {
    let buyer = SoftSigner::from_seed(BUYER, [1; 32]);
    let seller = SoftSigner::from_seed(SELLER, [2; 32]);
    let auditor = SoftSigner::from_seed(AUDITOR, [3; 32]);
    let mut verifier = StaticVerifier::new();
    verifier.add(&buyer);
    verifier.add(&seller);
    verifier.add(&auditor);
    Cast {
        buyer,
        seller,
        auditor,
        verifier,
    }
}

pub fn bindings() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("buyer".to_owned(), BUYER.to_owned()),
        ("seller".to_owned(), SELLER.to_owned()),
        ("auditor".to_owned(), AUDITOR.to_owned()),
    ])
}

pub struct Space {
    pub cast: Cast,
    pub genesis: Message,
    pub buyer: Instance,
    pub seller: Instance,
    pub auditor: Instance,
}

/// A genesis by the buyer, read by the seller and the auditor, before any join.
pub fn pending() -> Space {
    let cast = cast();
    let p = quote();
    let (genesis, buyer) =
        Instance::create(p.clone(), &cast.buyer, "buyer", &bindings(), None).expect("genesis");
    let seller = Instance::from_genesis(p.clone(), &genesis, SELLER, &cast.verifier, None)
        .expect("seller reads it");
    let auditor = Instance::from_genesis(p, &genesis, AUDITOR, &cast.verifier, None)
        .expect("auditor reads it");
    Space {
        cast,
        genesis,
        buyer,
        seller,
        auditor,
    }
}

/// The same space with every role joined and every join delivered to everyone.
pub fn open() -> Space {
    let mut s = pending();
    let j_seller = s.seller.join(&s.cast.seller).expect("seller joins");
    let j_auditor = s.auditor.join(&s.cast.auditor).expect("auditor joins");
    for m in [&j_seller, &j_auditor] {
        s.buyer.receive(m, &s.cast.verifier, 0);
    }
    s.seller.receive(&j_auditor, &s.cast.verifier, 0);
    s.auditor.receive(&j_seller, &s.cast.verifier, 0);
    s
}
