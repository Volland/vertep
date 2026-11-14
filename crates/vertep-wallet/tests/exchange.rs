//! The exchange engine end to end: role charts, the native protocol, the wallet's checks.

mod common;

use common::*;
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::testkit::Schedule;
use vertep_wallet::vocab::{PROTOCOL_VCE, Role};

#[test]
fn issue_and_present_end_to_end() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let verifier = w.agent();
    w.party(&verifier).wallet_mut().trust(&issuer);
    let x = issue(&mut w, &issuer, &holder, id_offer());
    assert_eq!(w.party(&issuer).state(Role::Issuer, &x).unwrap(), vec!["issued"]);
    let held = w.party(&holder).wallet().list().unwrap();
    assert_eq!(held.len(), 1, "{}", w.kit.jsonl());
    allow(&w, &holder, RulePolicy::refuse_all().allow(&verifier, &["nationality"]));
    let r = w
        .party(&verifier)
        .request(PROTOCOL_VCE, &holder, &["nationality", "name"], &[], None)
        .unwrap();
    w.kit.run(Schedule::honest());
    let v = w.party(&verifier).verified(&r).cloned().unwrap_or_else(|| panic!("{}", w.kit.jsonl()));
    assert_eq!(v.issuer, issuer);
    assert_eq!(v.disclosed.keys().cloned().collect::<Vec<_>>(), vec!["nationality".to_owned()]);
    assert_eq!(w.party(&verifier).state(Role::Verifier, &r).unwrap(), vec!["accepted"]);
    lean_accepts(&w);
}
