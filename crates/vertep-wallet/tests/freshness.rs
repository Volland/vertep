//! Possession before issuance, one issuance per offer, bound and fresh presentations, and
//! replay from the journal.

mod common;

use common::*;
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::history::Claims;
use vertep_wallet::testkit::Schedule;
use vertep_wallet::vocab::{PROTOCOL_VCE, Role};
use vertep_wallet::{Line, PresentationRequest};

fn msg_of(w: &World, f: impl Fn(&Line) -> Option<String>) -> String {
    w.history().lines().iter().find_map(f).expect("message")
}

fn issues(w: &World) -> usize {
    w.history().lines().iter().filter(|l| matches!(l, Line::Issue { .. })).count()
}

// @tg: verifies:: [[openspec:credential-exchange#Possession Before Issuance#Valid possession proof]] {level: integration}
#[test]
fn valid_possession_proof() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let x = issue(&mut w, &issuer, &holder, id_offer());
    let meta = w.party(&issuer).issued(&x).cloned().expect("issued");
    let holder_key = w.party(&holder).wallet().holder().unwrap().did_key_id();
    assert_eq!(meta.holder_key, holder_key);
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:credential-exchange#Possession Before Issuance#Another issuer's nonce refused]] {level: integration}
#[test]
fn another_issuers_nonce_refused() {
    let mut w = World::new();
    let issuer_a = w.agent();
    let issuer_b = w.agent();
    let holder = w.agent();
    // B offers too, but its offer is lost; A's possession proof is then replayed to B
    w.party(&issuer_b).offer(PROTOCOL_VCE, &holder, id_offer()).unwrap();
    w.kit.run(Schedule { drop: 100, ..Schedule::honest() });
    issue(&mut w, &issuer_a, &holder, id_offer());
    let pop = msg_of(&w, |l| match l {
        Line::ProvePossession { msg, .. } => Some(msg.clone()),
        _ => None,
    });
    w.kit.replay(&pop, &issuer_b);
    w.kit.run(Schedule::honest());
    let by_b = w
        .history()
        .lines()
        .iter()
        .filter(|l| matches!(l, Line::Issue { issuer, .. } if *issuer == issuer_b))
        .count();
    assert_eq!(by_b, 0);
    assert_eq!(issues(&w), 1);
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:credential-exchange#Possession Before Issuance#Reused nonce refused]] {level: integration}
#[test]
fn reused_nonce_refused() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let x = issue(&mut w, &issuer, &holder, id_offer());
    let pop = msg_of(&w, |l| match l {
        Line::ProvePossession { msg, .. } => Some(msg.clone()),
        _ => None,
    });
    w.kit.replay(&pop, &issuer);
    w.kit.run(Schedule::honest());
    assert_eq!(issues(&w), 1);
    assert_eq!(w.party(&issuer).state(Role::Issuer, &x).unwrap(), vec!["issued"]);
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:credential-exchange#Issuance At Most Once Per Offer#Redelivered request issues once]] {level: integration}
#[test]
fn redelivered_request_issues_once() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    w.party(&issuer).offer(PROTOCOL_VCE, &holder, id_offer()).unwrap();
    // deliver the offer, then the possession proof three times
    w.kit.run(Schedule { duplicate: 100, ..Schedule::honest() });
    let pop = msg_of(&w, |l| match l {
        Line::ProvePossession { msg, .. } => Some(msg.clone()),
        _ => None,
    });
    w.kit.replay(&pop, &issuer);
    w.kit.run(Schedule::honest());
    assert_eq!(issues(&w), 1);
    assert_eq!(w.party(&holder).wallet().list().unwrap().len(), 1);
    lean_accepts(&w);
}

/// An issued credential, a trusting verifier, and one accepted presentation.
fn presented(w: &mut World) -> (String, String, String, String) {
    let issuer = w.agent();
    let holder = w.agent();
    let verifier = w.agent();
    w.party(&verifier).wallet_mut().trust(&issuer);
    issue(w, &issuer, &holder, id_offer());
    allow(w, &holder, RulePolicy::refuse_all().allow("*", &["nationality"]));
    let r = w.party(&verifier).request(PROTOCOL_VCE, &holder, &["nationality"], &[], None).unwrap();
    w.kit.run(Schedule::honest());
    assert!(w.party(&verifier).verified(&r).is_some());
    (issuer, holder, verifier, r)
}

fn present_msg(w: &World) -> String {
    msg_of(w, |l| match l {
        Line::Present { msg, .. } => Some(msg.clone()),
        _ => None,
    })
}

fn refusals(w: &World, by: &str) -> usize {
    w.history()
        .lines()
        .iter()
        .filter(|l| matches!(l, Line::Verify { verifier, result, .. } if verifier == by && result == "refuse"))
        .count()
}

// @tg: verifies:: [[openspec:credential-exchange#Bound And Fresh Presentations#Replay to another verifier refused]] {level: integration}
#[test]
fn replay_to_another_verifier_refused() {
    let mut w = World::new();
    let (issuer, holder, _, _) = presented(&mut w);
    let other = w.agent();
    w.party(&other).wallet_mut().trust(&issuer);
    w.party(&other).request(PROTOCOL_VCE, &holder, &["nationality"], &[], None).unwrap();
    // hold the holder back: only the replay reaches the other verifier
    w.kit.run(Schedule { drop: 100, ..Schedule::honest() });
    let m = present_msg(&w);
    w.kit.replay(&m, &other);
    assert_eq!(refusals(&w, &other), 1);
    assert!(w.party(&other).exchanges().iter().all(|x| w.party(&other).verified(&x.id).is_none()));
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:credential-exchange#Bound And Fresh Presentations#Replayed presentation refused]] {level: integration}
#[test]
fn replayed_presentation_refused() {
    let mut w = World::new();
    let (_, _, verifier, _) = presented(&mut w);
    let m = present_msg(&w);
    w.kit.replay(&m, &verifier);
    let accepts = w
        .history()
        .lines()
        .iter()
        .filter(|l| matches!(l, Line::Verify { result, .. } if result == "accept"))
        .count();
    assert_eq!(accepts, 1);
    assert_eq!(refusals(&w, &verifier), 1);
    lean_accepts(&w);
}

fn request(verifier: &str, claims: &[&str]) -> PresentationRequest {
    PresentationRequest {
        verifier: verifier.to_owned(),
        nonce: "n-9".into(),
        claims: claims.iter().map(|c| (*c).to_owned()).collect(),
        types: vec![],
        purpose: None,
    }
}

// @tg: verifies:: [[openspec:credential-exchange#Bound And Fresh Presentations#Wrong holder key refused]] {level: integration}
#[test]
fn wrong_holder_key_refused() {
    use vertep_wallet::plugin::CredentialFormat;
    let mut w = World::new();
    let (_, holder, verifier, _) = presented(&mut w);
    let thief = w.agent();
    let (_, cred) = w.agent_parts(&holder).registry.store(vertep_wallet::vocab::STORE_SEALED).unwrap()
        .get(&w.party(&holder).wallet().list().unwrap()[0].digest).unwrap();
    // the thief has the credential bytes but presents with its own key
    let t = w.party(&thief).wallet();
    let names: Claims = ["nationality".to_owned()].into();
    let p = vertep_wallet::format::sdjwt::SdJwtVc
        .present(t.key_manager().as_ref(), &cred, &t.holder().unwrap(), &verifier, "n-9", &names)
        .unwrap();
    let err = w.party(&verifier).wallet().verify_presentation(&p, &request(&verifier, &["nationality"])).unwrap_err();
    assert_eq!(err.check(), "holder-binding");
}

// @tg: verifies:: [[openspec:credential-exchange#Bound And Fresh Presentations#Untrusted issuer refused]] {level: integration}
#[test]
fn untrusted_issuer_refused() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let verifier = w.agent(); // trusts nobody
    issue(&mut w, &issuer, &holder, id_offer());
    let digest = w.party(&holder).wallet().list().unwrap()[0].digest.clone();
    let names: Claims = ["nationality".to_owned()].into();
    let (_, p) = w.party(&holder).wallet().present(&digest, &names, &verifier, "n-9").unwrap();
    let err = w.party(&verifier).wallet().verify_presentation(&p, &request(&verifier, &["nationality"])).unwrap_err();
    assert_eq!(err.check(), "untrusted-issuer");
    assert!(err.to_string().contains(&issuer));
}

// @tg: verifies:: [[openspec:credential-exchange#Bound And Fresh Presentations#Unrequested claim refused]] {level: integration}
#[test]
fn unrequested_claim_refused() {
    let mut w = World::new();
    let (_, holder, verifier, _) = presented(&mut w);
    let digest = w.party(&holder).wallet().list().unwrap()[0].digest.clone();
    let names: Claims = ["nationality".to_owned(), "name".to_owned()].into();
    let (_, p) = w.party(&holder).wallet().present(&digest, &names, &verifier, "n-9").unwrap();
    let err = w.party(&verifier).wallet().verify_presentation(&p, &request(&verifier, &["nationality"])).unwrap_err();
    assert_eq!(err.check(), "unrequested-claim");
}

// @tg: verifies:: [[openspec:credential-exchange#Role Charts#Exchange replays from the journal]] {level: integration}
#[test]
fn exchange_replays_from_the_journal() {
    let mut w = World::new();
    let (_, holder, _, r) = presented(&mut w);
    let sent = w.kit.sent().len();
    let p = w.party(&holder);
    let actor = p.exchange(Role::Holder, &r).unwrap().actor.clone();
    let live = p.host().state(&actor).unwrap();
    p.host().deactivate(&actor);
    let replayed = p.host().state(&actor).unwrap();
    assert_eq!(replayed.terminated, live.terminated);
    assert_eq!(data(replayed.datamodel.clone()), data(live.datamodel.clone()));
    assert_eq!(p.state(Role::Holder, &r).unwrap(), vec!["presented"]);
    assert_eq!(w.kit.sent().len(), sent, "replay sent a message");
}
