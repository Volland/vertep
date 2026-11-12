//! The relay envelope, its JSON-LD shape, and the inbound admission pipeline.

use vertep_core::datamodel::{Event, EventType, Value};
use vertep_core::types::parse_declarations;
use vertep_federation::envelope::jsonld_graph;
use vertep_federation::ontology::{project_inbound, validate};
use vertep_federation::origin::{DeclaredPayloads, Untyped};
use vertep_federation::vocab::{DIDCOMM_PROCESSOR, TSP_PROCESSOR};
use vertep_federation::{
    AnonymousPolicy, Grant, GrantSet, InboundPolicy, InboundRecord, OpenedMessage, Refusal,
    RelayEnvelope, Timestamp, VerifiedOrigin, admit,
};

const ME: &str = "did:web:shop.example";
const ALICE: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

fn order_event() -> Event {
    let mut e = Event::named("order.placed", EventType::External);
    e.sendid = Some("s-17".into());
    e.origin = Some("did:example:spoofed".into());
    e.origintype = Some("urn:spoofed".into());
    e.data = Value::Object(vec![
        ("id".into(), Value::String("o-1".into())),
        ("amount".into(), Value::Number(12.5)),
        (
            "lines".into(),
            Value::Array(vec![Value::Number(1.0), Value::Bool(true), Value::Null]),
        ),
    ]);
    e
}

fn granted_policy() -> InboundPolicy {
    let mut grants = GrantSet::new();
    grants.add(Grant::new(ALICE, ME, ["order.*"]));
    InboundPolicy {
        actor: ME.into(),
        anonymous: AnonymousPolicy::Refuse,
        grants,
    }
}

#[test]
fn envelope_round_trips_and_never_carries_origin() {
    let env =
        RelayEnvelope::from_event("m-1", &order_event()).with_chart_version("urn:vertep:chart:abc");
    let bytes = env.to_bytes();
    let text = String::from_utf8(bytes.clone()).expect("utf8");
    assert!(
        !text.contains("spoofed"),
        "origin and origintype are not sent: {text}"
    );
    let back = RelayEnvelope::from_bytes(&bytes).expect("reads");
    assert_eq!(back, env);
    let ev = back.to_event();
    assert_eq!(
        ev.data,
        order_event().data,
        "member order and values survive"
    );
    assert_eq!(ev.sendid.as_deref(), Some("s-17"));
    assert_eq!(ev.origin, None);
}

#[test]
fn envelope_json_ld_conforms_to_its_shape() {
    let env = RelayEnvelope::from_event("m-1", &order_event())
        .with_chart_version("urn:vertep:chart:abc")
        .with_data_type("urn:vertep:type:sha256:00");
    let g = env.to_graph().expect("JSON-LD reads");
    let report = validate(&g);
    assert!(report.conforms(), "{report}");
    let nt = g.to_string();
    assert!(nt.contains("https://w3id.org/vertep/vt-id#RelayEnvelope"));
    assert!(nt.contains("http://www.w3.org/1999/02/22-rdf-syntax-ns#JSON"));
    assert!(nt.contains("\"order.placed\""));

    // A body that tries to set an origin gets no origin triple: the term is not in the context.
    let mut v: serde_json::Value = serde_json::from_slice(&env.to_bytes()).expect("json");
    v["origin"] = serde_json::json!("did:example:mallory");
    let g2 = jsonld_graph(&serde_json::to_vec(&v).expect("bytes")).expect("reads");
    assert!(!g2.to_string().contains("mallory"));
    assert!(validate(&g2).conforms());
}

// @tg: verifies:: [[openspec:relay-mailboxes#SCXML Event Envelope Mapping#Unknown fields from a newer sender]] {level: integration}
#[test]
fn unknown_fields_from_a_newer_sender_are_ignored() {
    let env = RelayEnvelope::from_event("m-2", &order_event());
    // Extension headers in front of the known members, as a newer sender would add them.
    let body = String::from_utf8(env.to_bytes()).expect("utf8");
    let extended = body.replacen(
        '{',
        r#"{"priority":"high","vt-ext:trace":{"span":"abc","hops":[1,2]},"#,
        1,
    );
    let read = RelayEnvelope::from_bytes(extended.as_bytes()).expect("required fields intact");
    assert_eq!(read, env);
    let admitted = admit(
        &granted_policy(),
        &OpenedMessage {
            envelope: read,
            origin: VerifiedOrigin::verified(ALICE, format!("{ALICE}#k"), TSP_PROCESSOR),
        },
        &Untyped,
        Timestamp::from_secs(0),
    )
    .expect("delivered");
    assert_eq!(admitted.event.name, "order.placed");

    // Missing required fields are not ignored.
    let mut broken: serde_json::Value = serde_json::from_str(&extended).expect("json");
    broken.as_object_mut().expect("object").remove("messageId");
    assert!(RelayEnvelope::from_bytes(&serde_json::to_vec(&broken).expect("bytes")).is_err());
}

// @tg: verifies:: [[openspec:relay-mailboxes#SCXML Event Envelope Mapping#Payload fails the receiver's type]] {level: integration}
#[test]
fn payload_that_fails_the_receivers_type_is_refused_and_recorded() {
    let decls = parse_declarations("event order.placed: { id: string, amount: decimal };")
        .expect("declarations");
    let types = DeclaredPayloads::new(decls);
    let mut bad = order_event();
    bad.data = Value::Object(vec![
        ("id".into(), Value::Number(1.0)),
        ("amount".into(), Value::String("lots".into())),
    ]);
    let msg = OpenedMessage {
        envelope: RelayEnvelope::from_event("m-3", &bad),
        origin: VerifiedOrigin::verified(ALICE, format!("{ALICE}#k"), DIDCOMM_PROCESSOR),
    };
    let refusal =
        admit(&granted_policy(), &msg, &types, Timestamp::from_secs(0)).expect_err("type mismatch");
    assert!(matches!(&refusal, Refusal::PayloadType { event, .. } if event == "order.placed"));
    assert!(
        refusal.notifies_sender(),
        "the sender gets error.communication"
    );
    let g = project_inbound(&InboundRecord::refused(
        ME,
        Some(&msg),
        refusal,
        Timestamp::from_secs(0),
    ))
    .expect("recorded");
    assert!(validate(&g).conforms());

    let mut good = order_event();
    good.data = Value::Object(vec![
        ("id".into(), Value::String("o-2".into())),
        ("amount".into(), Value::Number(3.0)),
    ]);
    let ok = OpenedMessage {
        envelope: RelayEnvelope::from_event("m-4", &good),
        origin: msg.origin.clone(),
    };
    admit(&granted_policy(), &ok, &types, Timestamp::from_secs(0)).expect("fits its type");
}

// @tg: verifies:: [[openspec:did-identity#Unauthenticated Event Marking#Anonymous message allowed]] {level: integration}
#[test]
fn anonymous_message_allowed_is_delivered_unverified() {
    let mut policy = granted_policy();
    policy.anonymous = AnonymousPolicy::Allow;
    let msg = OpenedMessage {
        envelope: RelayEnvelope::from_event("m-5", &order_event()),
        origin: VerifiedOrigin::anonymous(DIDCOMM_PROCESSOR),
    };
    let admitted = admit(&policy, &msg, &Untyped, Timestamp::from_secs(0)).expect("delivered");
    assert!(!admitted.origin.verified, "marked unverified for guards");
    assert!(
        !admitted.event.originverified,
        "_event marks the origin unverified"
    );
    assert_eq!(
        admitted.event.origin, None,
        "no sender DID in _event.origin"
    );
    assert_eq!(
        admitted.event.origintype.as_deref(),
        Some(DIDCOMM_PROCESSOR)
    );
    let g = project_inbound(&InboundRecord::accepted(
        ME,
        &admitted,
        Timestamp::from_secs(0),
    ))
    .expect("projects");
    assert!(
        g.to_string()
            .contains("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>")
    );
}

// @tg: verifies:: [[openspec:did-identity#Unauthenticated Event Marking#Anonymous message refused by policy]] {level: integration}
#[test]
fn anonymous_message_refused_by_policy_is_recorded() {
    let msg = OpenedMessage {
        envelope: RelayEnvelope::from_event("m-6", &order_event()),
        origin: VerifiedOrigin::anonymous(DIDCOMM_PROCESSOR),
    };
    let refusal = admit(&granted_policy(), &msg, &Untyped, Timestamp::from_secs(0))
        .expect_err("requires authentication");
    assert_eq!(refusal, Refusal::AnonymousRefused);
    let record = InboundRecord::refused(ME, Some(&msg), refusal, Timestamp::from_secs(0));
    let g = project_inbound(&record).expect("projects");
    let report = validate(&g);
    assert!(report.conforms(), "{report}");
    assert!(g.to_string().contains("anonymous sender refused"));
}

#[test]
fn verified_sender_sets_origin_and_the_verified_mark() {
    let msg = OpenedMessage {
        envelope: RelayEnvelope::from_event("m-7", &order_event()),
        origin: VerifiedOrigin::verified(ALICE, format!("{ALICE}#k"), TSP_PROCESSOR),
    };
    let admitted =
        admit(&granted_policy(), &msg, &Untyped, Timestamp::from_secs(0)).expect("delivered");
    assert_eq!(admitted.event.origin.as_deref(), Some(ALICE));
    assert_eq!(admitted.event.origintype.as_deref(), Some(TSP_PROCESSOR));
    assert!(admitted.origin.verified);
    let g = project_inbound(&InboundRecord::accepted(
        ME,
        &admitted,
        Timestamp::from_secs(0),
    ))
    .expect("projects");
    assert!(validate(&g).conforms());
    let nt = g.to_string();
    assert!(nt.contains(&format!(
        "<https://w3id.org/vertep/vt-id#verifiedOrigin> <{ALICE}>"
    )));
    assert!(nt.contains("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"));
}
