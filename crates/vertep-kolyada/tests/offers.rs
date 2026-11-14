//! Offers, discovery and the RDF bridge.
#![allow(missing_docs)]

mod common;

use common::{AUDITOR, BUYER, SELLER, cast, quote, quote_doc};
use oxrdf::Graph;
use serde_json::json;
use vertep_kolyada::bridge;
use vertep_kolyada::offer::{ServiceProtocol, offered_protocols, service_entry, verify_fetched};
use vertep_kolyada::{Offer, OfferBody};

fn offer(
    did: &str,
    role: &str,
    signer: &vertep_kolyada::testkit::SoftSigner,
    expires: Option<u64>,
) -> Offer {
    Offer::sign(
        OfferBody {
            protocol: quote().id().as_str().into(),
            role: role.into(),
            did: did.into(),
            expires_ms: expires,
            initiators: None,
        },
        signer,
    )
    .unwrap()
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Offers#Offer verified]] {level: unit}
#[test]
fn offer_verified() {
    let c = cast();
    let o = offer(SELLER, "seller", &c.seller, None);
    assert!(o.verify(&c.verifier).is_ok());
    let mut forged = o.clone();
    forged.body.role = "buyer".into();
    assert!(forged.verify(&c.verifier).is_err());
    // Another DID cannot sign an offer for this one.
    assert!(
        Offer::sign(
            OfferBody {
                did: SELLER.into(),
                ..o.body.clone()
            },
            &c.buyer
        )
        .is_err(),
        "a signer signs for its own DID only"
    );
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Offers#Expired offer refused]] {level: unit}
#[test]
fn expired_offer_refused() {
    let c = cast();
    let o = offer(SELLER, "seller", &c.seller, Some(1_000));
    assert!(o.is_valid_at(999));
    assert!(!o.is_valid_at(1_000));
    let open_ended = offer(SELLER, "seller", &c.seller, None);
    assert!(open_ended.is_valid_at(u64::MAX));
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Discovery#Discovery by service entry]] {level: unit}
#[test]
fn discovery_by_service_entry() {
    let entry = service_entry(
        SELLER,
        &[ServiceProtocol {
            protocol: quote().id().as_str().into(),
            roles: vec!["seller".into()],
        }],
    );
    let did_document = json!({ "id": SELLER, "service": [ { "id": "#x", "type": "LinkedDomains", "serviceEndpoint": "https://x" }, entry ] });
    let found = offered_protocols(&did_document);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].protocol, quote().id().as_str());
    assert_eq!(found[0].roles, ["seller"]);
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Discovery#Wrong document refused]] {level: unit}
#[test]
fn wrong_document_refused() {
    let listed = quote().id().as_str().to_owned();
    let bytes = quote_doc().to_string().into_bytes();
    assert!(verify_fetched(&listed, &bytes).is_ok());
    let mut other = quote_doc();
    other["transitions"][4]["to"] = json!("Cancelled");
    let err = verify_fetched(&listed, other.to_string().as_bytes()).unwrap_err();
    assert!(err.contains("expected"), "{err}");
}

// @tg: verifies:: [[openspec:agent-protocols#RDF Bridge#Who offers a role]] {level: integration}
#[test]
fn who_offers_a_role() {
    let c = cast();
    let mut graph = Graph::new();
    bridge::add_protocol(&mut graph, &quote());
    for (did, role, signer) in [
        (SELLER, "seller", &c.seller),
        (AUDITOR, "seller", &c.auditor),
        (BUYER, "buyer", &c.buyer),
    ] {
        bridge::add_offer(&mut graph, &offer(did, role, signer, None)).unwrap();
    }
    let store = oxigraph::store::Store::new().unwrap();
    for t in graph.iter() {
        store
            .insert(t.in_graph(oxrdf::GraphNameRef::DefaultGraph))
            .unwrap();
    }
    let query = format!(
        "PREFIX vk: <https://w3id.org/vertep/kolyada#>\n\
         SELECT ?did WHERE {{ ?did vk:offers ?o . ?o vk:protocol <{}> ; vk:role \"seller\" }} ORDER BY ?did",
        quote().id()
    );
    let results = oxigraph::sparql::SparqlEvaluator::new()
        .parse_query(&query)
        .unwrap()
        .on_store(&store)
        .execute()
        .unwrap();
    let mut dids = Vec::new();
    if let oxigraph::sparql::QueryResults::Solutions(solutions) = results {
        for s in solutions {
            dids.push(s.unwrap().get("did").unwrap().to_string());
        }
    }
    assert_eq!(dids, [format!("<{AUDITOR}>"), format!("<{SELLER}>")]);
}
