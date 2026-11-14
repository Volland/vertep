//! The protocol document, its identity, shapes, well-formedness and projection.
#![allow(missing_docs)]

mod common;

use common::{parse, quote, quote_doc};
use serde_json::{Value, json};
use vertep_kolyada::projection::Action;
use vertep_kolyada::{Protocol, ProtocolError};

fn parse_err(doc: &Value) -> ProtocolError {
    Protocol::parse(doc.to_string().as_bytes()).expect_err("the document is refused")
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Document#Valid document accepted]] {level: unit}
#[test]
fn valid_document_accepted() {
    let p = quote();
    assert_eq!(p.roles(), ["buyer", "seller", "auditor"]);
    assert_eq!(p.messages().len(), 5);
    assert_eq!(p.transitions().len(), 5);
    assert_eq!(p.initial(), "Idle");
    assert!(p.id().as_str().starts_with("urn:vertep:protocol:sha256:"));
    assert_eq!(p.limits().buffer_size, 4);
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Document#Foreign context refused]] {level: unit}
#[test]
fn foreign_context_refused() {
    let mut doc = quote_doc();
    doc["@context"] = json!("https://example.org/other.jsonld");
    match parse_err(&doc) {
        ProtocolError::Context(c) => assert!(c.contains("example.org/other.jsonld")),
        e => panic!("{e}"),
    }
    doc["@context"] =
        json!(["https://w3id.org/vertep/kolyada/1.0", { "x": "https://example.org/x" }]);
    assert!(matches!(parse_err(&doc), ProtocolError::Context(_)));
    doc["@context"] = json!({ "x": "https://example.org/x" });
    assert!(matches!(parse_err(&doc), ProtocolError::Context(_)));
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Document#Unknown term refused]] {level: unit}
#[test]
fn unknown_term_refused() {
    let mut doc = quote_doc();
    doc["owner"] = json!("did:example:someone");
    assert_eq!(parse_err(&doc), ProtocolError::UnknownTerm("owner".into()));
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Identity#Spellings share an identity]] {level: unit}
#[test]
fn spellings_share_an_identity() {
    let a = parse(&quote_doc());
    let mut doc = quote_doc();
    doc["roles"].as_array_mut().unwrap().reverse();
    doc["transitions"].as_array_mut().unwrap().reverse();
    doc["messages"].as_array_mut().unwrap().reverse();
    doc["final"].as_array_mut().unwrap().reverse();
    let b = parse(&doc);
    assert_eq!(a.id(), b.id());
}

// @tg: verifies:: [[openspec:agent-protocols#Protocol Identity#Any change changes the identity]] {level: unit}
#[test]
fn any_change_changes_the_identity() {
    let a = parse(&quote_doc());
    let mut doc = quote_doc();
    doc["transitions"][4]["to"] = json!("Cancelled");
    let b = parse(&doc);
    assert_ne!(a.id(), b.id());
}

// @tg: verifies:: [[openspec:agent-protocols#Payload Shapes#Payload validated]] {level: unit}
#[test]
fn payload_validated() {
    let p = quote();
    let shape = p.messages()["quote.request"].shape.as_ref().unwrap();
    assert!(
        shape
            .validate(&json!({ "item": "widget", "qty": 3 }))
            .is_ok()
    );
}

// @tg: verifies:: [[openspec:agent-protocols#Payload Shapes#Missing required field]] {level: unit}
#[test]
fn missing_required_field() {
    let p = quote();
    let shape = p.messages()["quote.request"].shape.as_ref().unwrap();
    let err = shape.validate(&json!({ "item": "widget" })).unwrap_err();
    assert_eq!(err.path, "qty");
}

// @tg: verifies:: [[openspec:agent-protocols#Payload Shapes#Shape outside the subset]] {level: unit}
#[test]
fn shape_outside_the_subset() {
    let mut doc = quote_doc();
    doc["messages"][0]["payload"]["sparql"] = json!("SELECT * WHERE {}");
    assert_eq!(parse_err(&doc), ProtocolError::UnknownTerm("sparql".into()));
    let mut doc = quote_doc();
    doc["messages"][0]["payload"]["fields"][0]["datatype"] = json!("dateTime");
    assert!(matches!(parse_err(&doc), ProtocolError::Shape(_)));
}

// @tg: verifies:: [[openspec:agent-protocols#Well-Formed Choreography#Two deciders refused]] {level: unit}
#[test]
fn two_deciders_refused() {
    let mut doc = quote_doc();
    doc["transitions"].as_array_mut().unwrap().push(json!(
        { "from": "Decided", "sender": "seller", "receivers": ["buyer"], "message": "receipt", "to": "Done" }
    ));
    match parse_err(&doc) {
        ProtocolError::WellFormed(e) => {
            assert_eq!(e.state.as_deref(), Some("Decided"));
            assert!(
                e.reason.contains("buyer") && e.reason.contains("seller"),
                "{}",
                e.reason
            );
        }
        e => panic!("{e}"),
    }
}

// @tg: verifies:: [[openspec:agent-protocols#Well-Formed Choreography#Unreachable end refused]] {level: unit}
#[test]
fn unreachable_end_refused() {
    let mut doc = quote_doc();
    doc["transitions"].as_array_mut().unwrap().push(json!(
        { "from": "Quoting", "sender": "seller", "receivers": ["buyer"], "message": "order", "to": "Stuck" }
    ));
    doc["transitions"].as_array_mut().unwrap().push(json!(
        { "from": "Stuck", "sender": "buyer", "receivers": ["seller"], "message": "cancel", "to": "Stuck" }
    ));
    match parse_err(&doc) {
        ProtocolError::WellFormed(e) => assert_eq!(e.state.as_deref(), Some("Stuck")),
        e => panic!("{e}"),
    }
}

// @tg: verifies:: [[openspec:agent-protocols#Role Projection#Projection of a request and reply]] {level: unit}
#[test]
fn projection_of_a_request_and_reply() {
    let p = quote();
    let seller = p.project("seller").unwrap();
    let first = seller.steps.iter().find(|s| s.from == "Idle").unwrap();
    assert_eq!(
        first.action,
        Action::Receive {
            message: "quote.request".into(),
            sender: "buyer".into()
        }
    );
    let then = seller.steps.iter().find(|s| s.from == "Quoting").unwrap();
    assert!(matches!(&then.action, Action::Send { message, .. } if message == "quote"));
    let buyer = p.interface("buyer").unwrap();
    assert_eq!(
        buyer.accepts["Quoting"],
        [("quote".to_owned(), "seller".to_owned())]
    );
    let auditor = p.project("auditor").unwrap();
    assert!(
        auditor
            .steps
            .iter()
            .any(|s| s.from == "Idle" && s.action == Action::Silent)
    );
    assert!(p.project("nobody").is_none());
}

// @tg: verifies:: [[openspec:agent-protocols#Role Projection#Projection is deterministic]] {level: unit}
#[test]
fn projection_is_deterministic() {
    let p = quote();
    let first: Vec<_> = ["buyer", "seller", "auditor"]
        .iter()
        .map(|r| p.project(r).unwrap())
        .collect();
    let second: Vec<_> = ["auditor", "seller", "buyer"]
        .iter()
        .rev()
        .map(|r| p.project(r).unwrap())
        .collect();
    assert_eq!(first, second);
}
