//! The wallet actor, the sealed store, holder binding and the credential lifecycle.

mod common;

use std::sync::Arc;

use common::*;
use vertep_federation::KeyManager;
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::store::RecordBackend;
use vertep_wallet::testkit::Schedule;
use vertep_wallet::vocab::{PROTOCOL_VCE, Role};
use vertep_wallet::{Line, Wallet, WalletConfig};

fn issued(w: &mut World) -> (String, String, String) {
    let issuer = w.agent();
    let holder = w.agent();
    issue(w, &issuer, &holder, id_offer());
    let digest = w.party(&holder).wallet().list().unwrap()[0].digest.clone();
    (issuer, holder, digest)
}

// @tg: verifies:: [[openspec:agent-wallet#Wallet Actor#Wallet survives a restart]] {level: integration}
#[test]
fn wallet_survives_a_restart() {
    let mut w = World::new();
    let (_, holder, digest) = issued(&mut w);
    let before = w.party(&holder).wallet().list().unwrap();
    // a new engine over the same journal and records: the node restarted
    let a = w.agent_parts(&holder);
    let mut again = w.engine(a, None, false);
    assert_eq!(again.wallet().list().unwrap(), before);
    let actor = again.wallet_actor().to_owned();
    let dm = again.host().state(&actor).unwrap().datamodel;
    let json = vertep_agent::json::to_json(&dm).to_string();
    assert!(json.contains(&digest), "{json}");
}

// @tg: verifies:: [[openspec:agent-wallet#Wallet Actor#One wallet per identity]] {level: integration}
#[test]
fn one_wallet_per_identity() {
    let mut w = World::new();
    let holder = w.agent();
    let a = w.agent_parts(&holder);
    let km: Arc<dyn KeyManager> = a.km.clone();
    let wallet = Wallet::new(
        WalletConfig::did_key(km.as_ref(), &a.key).unwrap(),
        km,
        a.registry.clone(),
        w.keys(),
        Arc::new(vertep_federation::RuntimeClock(Arc::new(w.clock.clone()))),
    );
    let err = vertep_wallet::Exchanges::builder(Arc::new(w.clock.clone()))
        .journal(a.journal.clone())
        .create(wallet, w.kit.wire())
        .unwrap_err();
    assert_eq!(err.check(), "exists");
    assert!(err.to_string().contains("wallet"), "{err}");
}

// @tg: verifies:: [[openspec:agent-wallet#Sealed Credential Store#Stored credential is encrypted]] {level: integration}
#[test]
fn stored_credential_is_encrypted() {
    let mut w = World::new();
    let (_, holder, _) = issued(&mut w);
    let records = &w.agent_parts(&holder).records;
    for n in records.names().unwrap() {
        let bytes = records.read(&n).unwrap().unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("1990-05-01") && !text.contains("Olena"), "{n} holds a claim value");
    }
}

// @tg: verifies:: [[openspec:agent-wallet#Sealed Credential Store#Graph holds metadata only]] {level: integration}
#[test]
fn graph_holds_metadata_only() {
    let mut w = World::new();
    let (issuer, holder, digest) = issued(&mut w);
    let p = w.party(&holder);
    let actor = p.wallet_actor().to_owned();
    let graph = p.host().graph(&actor).unwrap().to_string();
    let state = vertep_agent::json::to_json(&p.host().state(&actor).unwrap().datamodel).to_string();
    let journal = format!("{:?}", p.host().journal(&actor));
    for text in [&graph, &state, &journal] {
        assert!(!text.contains("1990-05-01") && !text.contains("Olena"), "claim value in {text}");
    }
    assert!(state.contains(&digest) && state.contains(&issuer), "{state}");
}

// @tg: verifies:: [[openspec:agent-wallet#Sealed Credential Store#Tampered record refused]] {level: integration}
#[test]
fn tampered_record_refused() {
    let mut w = World::new();
    let (_, holder, digest) = issued(&mut w);
    let records = w.agent_parts(&holder).records.clone();
    let name = format!("cred-{}", digest.trim_start_matches("sha256:"));
    let mut bytes = records.read(&name).unwrap().unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    records.write(&name, &bytes).unwrap();
    let all: vertep_wallet::history::Claims = ["name".to_owned()].into();
    let err = w
        .party(&holder)
        .wallet()
        .present(&digest, &all, "did:example:v", "n")
        .unwrap_err();
    assert_eq!(err.check(), "integrity");
}

// @tg: verifies:: [[openspec:agent-wallet#Holder Binding Keys#Binding key from the key manager]] {level: integration}
#[test]
fn binding_key_from_the_key_manager() {
    let mut w = World::new();
    let (_, holder, _) = issued(&mut w);
    let meta = w.party(&holder).wallet().list().unwrap()[0].clone();
    let a = w.agent_parts(&holder);
    let public = a.km.public_key(&a.key).unwrap();
    assert_eq!(meta.holder_key, vertep_federation::resolver::key::did_key(&public));
    let pop_key = w
        .history()
        .lines()
        .into_iter()
        .find_map(|l| match l {
            Line::ProvePossession { key, .. } => Some(key),
            _ => None,
        })
        .unwrap();
    assert_eq!(pop_key, meta.holder_key);
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Lifecycle#Expired credential not presented]] {level: integration}
#[test]
fn expired_credential_not_presented() {
    let mut w = World::new();
    let issuer = w.agent();
    let holder = w.agent();
    let verifier = w.agent();
    w.party(&verifier).wallet_mut().trust(&issuer);
    let mut offer = id_offer();
    offer.valid_until = Some(T0 + DAY);
    issue(&mut w, &issuer, &holder, offer);
    allow(&w, &holder, RulePolicy::refuse_all().allow("*", &["name"]));
    w.clock.advance(std::time::Duration::from_millis(2 * DAY as u64));
    let r = w.party(&verifier).request(PROTOCOL_VCE, &holder, &["name"], &[], None).unwrap();
    w.kit.run(Schedule::honest());
    assert!(w.party(&verifier).verified(&r).is_none());
    assert_eq!(w.party(&holder).state(Role::Holder, &r).unwrap(), vec!["refused"]);
    assert!(!w.history().lines().iter().any(|l| matches!(l, Line::Present { .. })));
    let digest = w.party(&holder).wallet().list().unwrap()[0].digest.clone();
    let names: vertep_wallet::history::Claims = ["name".to_owned()].into();
    assert_eq!(
        w.party(&holder).wallet().present(&digest, &names, &verifier, "n").unwrap_err().check(),
        "validity"
    );
}

// @tg: verifies:: [[openspec:agent-wallet#Credential Lifecycle#Deleted credential gone]] {level: integration}
#[test]
fn deleted_credential_gone() {
    let mut w = World::new();
    let (_, holder, digest) = issued(&mut w);
    w.party(&holder).delete(&digest).unwrap();
    assert!(w.party(&holder).wallet().list().unwrap().is_empty());
    let records = &w.agent_parts(&holder).records;
    assert!(!records.names().unwrap().iter().any(|n| n.contains(digest.trim_start_matches("sha256:"))));
    let p = w.party(&holder);
    let actor = p.wallet_actor().to_owned();
    let journal = format!("{:?}", p.host().journal(&actor));
    assert!(journal.contains("credential.deleted") && journal.contains(&digest));
    assert!(!journal.contains("Olena"));
}
