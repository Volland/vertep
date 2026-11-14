//! Consent before disclosure: the policy bounds the decide state's candidates.

mod common;

use std::sync::Arc;

use common::*;
use futures::future::BoxFuture;
use vertep_agent::decider::{Decider, DeciderKind, DecisionOutcome, DecisionRequest};
use vertep_agent::journal::AgentInput;
use vertep_wallet::consent::{RulePolicy, disclose, offered_options};
use vertep_wallet::plugin::DisclosureOption;
use vertep_wallet::testkit::Schedule;
use vertep_wallet::vocab::{PROTOCOL_VCE, Role};
use vertep_wallet::Line;

fn setup(policy: impl Fn(&str) -> RulePolicy, decider: Option<Arc<dyn Decider>>) -> (World, String, String, String) {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent_with(decider, |_| {});
    let verifier = w.agent();
    w.party(&verifier).wallet_mut().trust(&issuer);
    issue(&mut w, &issuer, &holder, id_offer());
    allow(&w, &holder, policy(&verifier));
    (w, issuer, holder, verifier)
}

// @tg: verifies:: [[openspec:agent-wallet#Consent Before Disclosure#Policy narrows the disclosure]] {level: integration}
#[test]
fn policy_narrows_the_disclosure() {
    let (mut w, _, holder, verifier) = setup(|v| RulePolicy::refuse_all().allow(v, &["birthDate"]), None);
    let r = w.party(&verifier).request(PROTOCOL_VCE, &holder, &["name", "birthDate"], &[], None).unwrap();
    w.kit.run(Schedule::honest());
    let v = w.party(&verifier).verified(&r).cloned().expect("accepted");
    assert_eq!(v.disclosed.keys().collect::<Vec<_>>(), vec!["birthDate"]);
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:agent-wallet#Consent Before Disclosure#Policy refuses]] {level: integration}
#[test]
fn policy_refuses() {
    let (mut w, _, holder, verifier) = setup(|_| RulePolicy::refuse_all(), None);
    let r = w.party(&verifier).request(PROTOCOL_VCE, &holder, &["name"], &[], None).unwrap();
    w.kit.run(Schedule::honest());
    assert!(!w.history().lines().iter().any(|l| matches!(l, Line::Present { .. })));
    assert_eq!(w.party(&verifier).state(Role::Verifier, &r).unwrap(), vec!["refused"]);
    lean_accepts(&w);
}

/// An "LLM" that discloses everything it was asked for, whatever the policy allowed.
struct Greedy;

impl Decider for Greedy {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Llm
    }
    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        let opts = offered_options(&request);
        let requested = request
            .context
            .get("requested")
            .map(vertep_agent::json::to_json)
            .and_then(|j| serde_json::from_value(j).ok())
            .unwrap_or_default();
        let outcome = disclose(&DisclosureOption {
            credential: opts.first().map(|o| o.credential.clone()).unwrap_or_default(),
            claims: requested,
        });
        Box::pin(async move { outcome })
    }
}

// @tg: verifies:: [[openspec:agent-wallet#Consent Before Disclosure#LLM decider cannot exceed the policy]] {level: integration}
#[test]
fn llm_decider_cannot_exceed_the_policy() {
    let (mut w, _, holder, verifier) =
        setup(|v| RulePolicy::refuse_all().allow(v, &["nationality"]), Some(Arc::new(Greedy)));
    let r = w
        .party(&verifier)
        .request(PROTOCOL_VCE, &holder, &["nationality", "name"], &[], None)
        .unwrap();
    w.kit.run(Schedule::honest());
    assert!(w.party(&verifier).verified(&r).is_none());
    assert!(!w.history().lines().iter().any(|l| matches!(l, Line::Present { .. })));
    let p = w.party(&holder);
    let actor = p.exchange(Role::Holder, &r).unwrap().actor.clone();
    let st = p.host().state(&actor).unwrap();
    let reason = data(st.datamodel)["reason"].clone();
    assert_eq!(reason, serde_json::json!("out-of-bounds"));
    assert!(p
        .host()
        .journal(&actor)
        .iter()
        .any(|e| matches!(&e.input, AgentInput::Decision { event, .. } if event == "consent.disclose")));
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:agent-wallet#Consent Before Disclosure#Consent is journaled]] {level: integration}
#[test]
fn consent_is_journaled() {
    let (mut w, _, holder, verifier) = setup(|v| RulePolicy::refuse_all().allow(v, &["name"]), None);
    let r = w.party(&verifier).request(PROTOCOL_VCE, &holder, &["name"], &[], None).unwrap();
    w.kit.run(Schedule::honest());
    let p = w.party(&holder);
    let actor = p.exchange(Role::Holder, &r).unwrap().actor.clone();
    let journal = p.host().journal(&actor);
    let decision = journal
        .iter()
        .position(|e| matches!(&e.input, AgentInput::Decision { event, .. } if event == "consent.disclose"))
        .expect("the consent decision is journaled");
    // replay to just after the decision: the chart has left consent with the disclosed claims
    let at = p.host().state_at(&actor, journal[decision].position).unwrap();
    let dm = data(at.datamodel);
    assert_eq!(dm["peer"], serde_json::json!(verifier));
    assert_eq!(dm["disclosed"], serde_json::json!(["name"]));
    assert!(!dm["nonce"].as_str().unwrap_or_default().is_empty());
    // and the presentation came after it
    let lines = w.history().lines();
    let c = lines.iter().position(|l| matches!(l, Line::Consent { .. })).unwrap();
    let pr = lines.iter().position(|l| matches!(l, Line::Present { .. })).unwrap();
    assert!(c < pr);
}
