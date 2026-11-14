//! A tour of Kolyada: a three-party quote protocol from document to completed instance.
//!
//! A buyer asks a seller for a quote, the seller answers (an auditor sees the quote), the buyer
//! orders, the seller sends a receipt. The tour prints the protocol's identity and the seller's
//! interface, opens a space with a signed genesis and countersigned joins, runs the
//! conversation, shows a forged message refused with a proof anyone can re-check, and signs an
//! offer for discovery.
//!
//! Run with `cargo run -p vertep-kolyada --example quote --features testkit`.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use vertep_kolyada::offer::{OfferBody, service_entry};
use vertep_kolyada::testkit::{SoftSigner, StaticVerifier};
use vertep_kolyada::{
    Body, Event, Instance, Message, Offer, Protocol, SendError, ServiceProtocol, Violation,
};

const QUOTE: &str = r#"{
  "@context": "https://w3id.org/vertep/kolyada/1.0",
  "type": "Protocol",
  "version": "1.0",
  "roles": ["buyer", "seller", "auditor"],
  "messages": [
    { "id": "quote.request", "payload": { "fields": [
      { "name": "item", "datatype": "string", "min": 1, "max": 1 },
      { "name": "qty", "datatype": "integer", "min": 1, "max": 1 } ] } },
    { "id": "quote", "payload": { "fields": [
      { "name": "price", "datatype": "number", "min": 1, "max": 1 },
      { "name": "currency", "datatype": "string", "in": ["EUR", "USD"] } ] } },
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
}"#;

const BUYER: &str = "did:example:buyer";
const SELLER: &str = "did:example:seller";
const AUDITOR: &str = "did:example:auditor";

fn applied(events: &[Event]) -> String {
    events
        .iter()
        .map(|e| match e {
            Event::Applied(h) => format!("applied {}", &h[..12]),
            Event::Held(h) => format!("held {}", &h[..12]),
            Event::Duplicate(h) => format!("duplicate {}", &h[..12]),
            Event::Refused { reason, .. } => format!("refused: {}", reason.as_str()),
            Event::Stalled { .. } => "stalled".to_owned(),
            Event::Equivocation(_) => "equivocation".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. The document and its identity: the hash of its RDF graph.
    let protocol = Arc::new(Protocol::parse(QUOTE.as_bytes())?);
    println!("protocol {}", protocol.id().as_str());

    // 2. The seller's interface: what it accepts, state by state, and from whom.
    let interface = protocol.interface("seller").ok_or("no seller role")?;
    for (state, accepts) in &interface.accepts {
        for (message, sender) in accepts {
            println!("  seller accepts in {state}: {message} from {sender}");
        }
    }

    // 3. Three agents with keys. In Vertep the Signer and Verifier come from the key manager
    //    and the DID resolver; the testkit's are in memory.
    let buyer = SoftSigner::from_seed(BUYER, [1; 32]);
    let seller = SoftSigner::from_seed(SELLER, [2; 32]);
    let auditor = SoftSigner::from_seed(AUDITOR, [3; 32]);
    let mut verifier = StaticVerifier::new();
    for s in [&buyer, &seller, &auditor] {
        verifier.add(s);
    }

    // 4. A space: the buyer signs a genesis binding a DID to every role; the others read it
    //    and countersign with a join. No role may speak before its join.
    let bindings = BTreeMap::from([
        ("buyer".to_owned(), BUYER.to_owned()),
        ("seller".to_owned(), SELLER.to_owned()),
        ("auditor".to_owned(), AUDITOR.to_owned()),
    ]);
    let (genesis, mut b) = Instance::create(protocol.clone(), &buyer, "buyer", &bindings, None)?;
    let mut s = Instance::from_genesis(protocol.clone(), &genesis, SELLER, &verifier, None)
        .map_err(|r| r.as_str())?;
    let mut a = Instance::from_genesis(protocol.clone(), &genesis, AUDITOR, &verifier, None)
        .map_err(|r| r.as_str())?;
    println!("instance {}", b.id());
    let joins = [s.join(&seller)?, a.join(&auditor)?];
    for j in &joins {
        for i in [&mut b, &mut s, &mut a] {
            if j.body.sender != bindings[i.local_role()] {
                i.receive(j, &verifier, 0);
            }
        }
    }
    println!("status after the joins: {:?}", b.status());

    // 5. The conversation. Each message is signed, names its dependencies, and is judged by
    //    the receiver's projection: the role gives the authority, no grant is needed.
    let req = b.send(
        &buyer,
        "quote.request",
        json!({"item": "lantern", "qty": 2}),
        &["seller"],
    )?;
    println!(
        "seller <- quote.request: {}",
        applied(&s.receive(&req, &verifier, 0))
    );
    let quote = s.send(
        &seller,
        "quote",
        json!({"price": 18.5, "currency": "EUR"}),
        &["buyer", "auditor"],
    )?;
    println!(
        "buyer  <- quote: {}",
        applied(&b.receive(&quote, &verifier, 0))
    );
    println!(
        "auditor <- quote: {}",
        applied(&a.receive(&quote, &verifier, 0))
    );
    // The quote depends on the request, which was sealed to the seller only. The auditor is
    // told its hash alone; the held quote is released, and the auditor never reads the request.
    let released = a.observe_hidden(&req.hash(), &verifier);
    println!(
        "auditor <- hash of the hidden request: {}",
        applied(&released)
    );

    // 6. The auditor's projection never sends `order`: its own send path refuses.
    match a.send(&auditor, "order", json!({}), &["seller"]) {
        Err(SendError::Refused(r)) => {
            println!("auditor -> order: refused before sending ({})", r.as_str())
        }
        other => println!("auditor -> order: {other:?}"),
    }

    // 7. A seller that skips ahead signs a receipt before any order. The buyer refuses it and
    //    keeps the signed message as a violation that anyone can re-check.
    let forged = Message::sign(
        Body {
            protocol: protocol.id().as_str().to_owned(),
            instance: Some(b.id().to_owned()),
            sender: SELLER.to_owned(),
            role: "seller".to_owned(),
            seq: 3,
            deps: b.heads(),
            recipients: vec!["buyer".to_owned()],
            kind: "receipt".to_owned(),
            payload: json!({}),
        },
        &seller,
    )?;
    for e in b.receive(&forged, &verifier, 0) {
        if let Event::Refused {
            reason, message, ..
        } = e
        {
            let v = Violation::new(&reason, *message);
            println!(
                "buyer  <- receipt: refused ({}); violation re-checks: {}",
                reason.as_str(),
                v.verify(&b, &verifier)
            );
        }
    }

    // 8. The protocol completes.
    let order = b.send(&buyer, "order", json!({}), &["seller"])?;
    println!(
        "seller <- order: {}",
        applied(&s.receive(&order, &verifier, 0))
    );
    let receipt = s.send(&seller, "receipt", json!({}), &["buyer"])?;
    println!(
        "buyer  <- receipt: {}",
        applied(&b.receive(&receipt, &verifier, 0))
    );
    println!(
        "complete: buyer {}, seller {}",
        b.is_complete(),
        s.is_complete()
    );

    // 9. Discovery: the seller signs an offer and lists the protocol in its DID document.
    let offer = Offer::sign(
        OfferBody {
            protocol: protocol.id().as_str().to_owned(),
            role: "seller".to_owned(),
            did: SELLER.to_owned(),
            expires_ms: None,
            initiators: None,
        },
        &seller,
    )?;
    println!("offer verifies: {}", offer.verify(&verifier).is_ok());
    let entry = service_entry(
        SELLER,
        &[ServiceProtocol {
            protocol: protocol.id().as_str().to_owned(),
            roles: vec!["seller".to_owned()],
        }],
    );
    println!("service entry: {entry}");
    Ok(())
}
