//! Bitstring Status List: revocation and unreachable lists.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use vertep_wallet::PresentationRequest;
use vertep_wallet::history::Claims;
use vertep_wallet::status::StatusList;
use vertep_wallet::vocab::PROTOCOL_VCE;

const LIST: &str = "https://issuer.example/status/1";

fn issued_with_status(w: &mut World) -> (String, String, String, Arc<Mutex<StatusList>>, String) {
    let issuer = w.agent();
    let holder = w.agent();
    let verifier = w.agent();
    w.party(&verifier).wallet_mut().trust(&issuer);
    let list = Arc::new(Mutex::new(StatusList::new(LIST, "revocation")));
    let mut offer = id_offer();
    offer.status = Some(list.lock().unwrap().allocate());
    let x = issue(w, &issuer, &holder, offer);
    publish(w, &issuer, &list);
    (issuer, holder, verifier, list, x)
}

fn publish(w: &mut World, issuer: &str, list: &Arc<Mutex<StatusList>>) {
    let p = w.party(issuer);
    let wallet = p.wallet();
    let cred = list
        .lock()
        .unwrap()
        .publish(wallet.key_manager().as_ref(), &wallet.signing().unwrap(), wallet.now(), DAY)
        .unwrap();
    w.status.publish(LIST, cred);
}

fn presentation(w: &mut World, holder: &str, verifier: &str) -> (vertep_wallet::Presentation, PresentationRequest) {
    let digest = w.party(holder).wallet().list().unwrap()[0].digest.clone();
    let names: Claims = ["nationality".to_owned()].into();
    let (_, p) = w.party(holder).wallet().present(&digest, &names, verifier, "n-1").unwrap();
    let req = PresentationRequest {
        verifier: verifier.to_owned(),
        nonce: "n-1".into(),
        claims: names,
        types: vec![],
        purpose: None,
    };
    (p, req)
}

#[test]
fn unrevoked_credential_accepted() {
    let mut w = World::new();
    let (_, holder, verifier, _, _) = issued_with_status(&mut w);
    let (p, req) = presentation(&mut w, &holder, &verifier);
    w.party(&verifier).wallet().verify_presentation(&p, &req).expect("valid while not revoked");
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Status#Revoked credential refused]] {level: integration}
#[test]
fn revoked_credential_refused() {
    let mut w = World::new();
    let (issuer, holder, verifier, list, x) = issued_with_status(&mut w);
    let (p, req) = presentation(&mut w, &holder, &verifier);
    let l = list.clone();
    w.party(&issuer).revoke(&x, |s| l.lock().unwrap().revoke(s.index)).unwrap();
    publish(&mut w, &issuer, &list);
    let err = w.party(&verifier).wallet().verify_presentation(&p, &req).unwrap_err();
    assert_eq!(err.check(), "revoked");
    // and the holder no longer offers it
    assert!(w.party(&holder).wallet().live().unwrap().is_empty());
    let r = w.party(&verifier).request(PROTOCOL_VCE, &holder, &["nationality"], &[], None).unwrap();
    w.kit.run(vertep_wallet::testkit::Schedule::honest());
    assert!(w.party(&verifier).verified(&r).is_none());
    lean_accepts(&w);
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Status#Unreachable status fails closed]] {level: integration}
#[test]
fn unreachable_status_fails_closed() {
    let mut w = World::new();
    let (_, holder, verifier, _, _) = issued_with_status(&mut w);
    let (p, req) = presentation(&mut w, &holder, &verifier);
    w.status.withdraw(LIST);
    let err = w.party(&verifier).wallet().verify_presentation(&p, &req).unwrap_err();
    assert_eq!(err.check(), "status");
    assert!(err.to_string().contains(LIST), "{err}");
}
