//! Grant credentials held in a wallet, presented over an exchange, and enforced before the
//! journal by the federation's inbound check.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use vertep_runtime::Clock as _;

use common::*;
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_federation::origin::Untyped;
use vertep_federation::vocab::GRANT_CREDENTIAL_TYPE;
use vertep_federation::{
    InboundPolicy, OpenedMessage, Refusal, RelayEnvelope, ResolverRegistry, Timestamp,
    VerifiedOrigin, admit, verify_grant_credential,
};
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::grants::{PresentedGrants, grant_offer};
use vertep_wallet::status::StatusList;
use vertep_wallet::testkit::Schedule;
use vertep_wallet::vocab::{PROTOCOL_VCE, STORE_SEALED};

const LIST: &str = "https://issuer.example/status/grants";

struct Setup {
    w: World,
    issuer: String,
    agent: String,
    target: String,
    list: Arc<Mutex<StatusList>>,
    offer_x: String,
}

fn publish(w: &mut World, issuer: &str, list: &Arc<Mutex<StatusList>>) {
    let p = w.party(issuer);
    let wallet = p.wallet();
    let cred = list
        .lock()
        .unwrap()
        .publish(
            wallet.key_manager().as_ref(),
            &wallet.signing().unwrap(),
            wallet.now(),
            DAY,
        )
        .unwrap();
    w.status.publish(LIST, cred);
}

/// An issuer the target trusts issues `agent` a grant credential for `order.*` on `target`.
fn setup() -> Setup {
    let mut w = World::new();
    let issuer = w.agent();
    let agent = w.agent();
    let target = w.agent();
    w.party(&target).wallet_mut().trust(&issuer);
    let list = Arc::new(Mutex::new(StatusList::new(LIST, "revocation")));
    let status = list.lock().unwrap().allocate();
    let offer_x = issue(
        &mut w,
        &issuer,
        &agent,
        grant_offer(&target, &["order.*"], None, Some(status)),
    );
    publish(&mut w, &issuer, &list);
    allow(
        &w,
        &agent,
        RulePolicy::refuse_all().allow(&target, &["target", "events"]),
    );
    Setup {
        w,
        issuer,
        agent,
        target,
        list,
        offer_x,
    }
}

/// The agent presents its grant credential to the target; the target installs the grant.
fn present(s: &mut Setup, grants: &mut PresentedGrants) -> Option<vertep_federation::Grant> {
    let r =
        s.w.party(&s.target)
            .request(
                PROTOCOL_VCE,
                &s.agent,
                &["target", "events"],
                &[GRANT_CREDENTIAL_TYPE],
                Some("grant"),
            )
            .unwrap();
    s.w.kit.run(Schedule::honest());
    let v = s.w.party(&s.target).verified(&r).cloned()?;
    Some(grants.install(&v, &s.target).unwrap())
}

fn opened(sender: &str, name: &str) -> OpenedMessage {
    let mut e = Event::named(name, EventType::External);
    e.data = Value::Object(vec![("id".into(), Value::String("o-1".into()))]);
    OpenedMessage {
        envelope: RelayEnvelope::from_event(format!("m-{name}"), &e),
        origin: VerifiedOrigin::verified(sender, format!("{sender}#k"), "urn:test:processor"),
    }
}

fn policy(target: &str, grants: &PresentedGrants) -> InboundPolicy {
    InboundPolicy {
        actor: target.to_owned(),
        anonymous: Default::default(),
        grants: grants.grant_set(),
    }
}

// @tg: verifies:: [[openspec:did-identity#Grant Credentials From A Wallet#Grant presented over an exchange]] {level: integration}
#[test]
fn grant_presented_over_an_exchange() {
    let mut s = setup();
    let mut grants = PresentedGrants::new();
    let g = present(&mut s, &mut grants).expect("the target accepted the grant credential");
    assert_eq!(g.grantee, s.agent);
    assert_eq!(g.issuer.as_deref(), Some(s.issuer.as_str()));
    let admitted = admit(
        &policy(&s.target, &grants),
        &opened(&s.agent, "order.placed"),
        &Untyped,
        Timestamp(s.w.clock.now_ms()),
    )
    .expect("the granted event is delivered");
    assert_eq!(admitted.event.name, "order.placed");
    // the credential the wallet holds is still a grant credential for the federation API
    let held = s.w.party(&s.agent).wallet().list().unwrap();
    let (_, cred) =
        s.w.agent_parts(&s.agent)
            .registry
            .store(STORE_SEALED)
            .unwrap()
            .get(&held[0].digest)
            .unwrap();
    let vc: serde_json::Value = serde_json::from_str(&cred.raw).unwrap();
    let registry = ResolverRegistry::with_builtin(None, Duration::from_secs(1));
    let fed = verify_grant_credential(
        &vc,
        &s.target,
        std::slice::from_ref(&s.issuer),
        &registry,
        Timestamp(s.w.clock.now_ms()),
    )
    .unwrap();
    assert_eq!(fed.events, vec!["order.*".to_owned()]);
    lean_accepts(&s.w);
}

// @tg: verifies:: [[openspec:did-identity#Grant Credentials From A Wallet#Presented grant enforced before the journal]] {level: integration}
#[test]
fn presented_grant_enforced_before_the_journal() {
    let mut s = setup();
    let mut grants = PresentedGrants::new();
    present(&mut s, &mut grants).unwrap();
    // the target's actor: only admitted events reach its journal
    let actor = s.w.party(&s.target).wallet_actor().to_owned();
    let before = s.w.party(&s.target).host().journal(&actor).len();
    let refusal = admit(
        &policy(&s.target, &grants),
        &opened(&s.agent, "admin.reset"),
        &Untyped,
        Timestamp(s.w.clock.now_ms()),
    )
    .expect_err("outside the grant");
    assert!(matches!(refusal, Refusal::NotGranted(_)));
    assert_eq!(s.w.party(&s.target).host().journal(&actor).len(), before);
}

// @tg: verifies:: [[openspec:did-identity#Grant Credentials From A Wallet#Revoked grant credential refused]] {level: integration}
#[test]
fn revoked_grant_credential_refused() {
    let mut s = setup();
    let mut grants = PresentedGrants::new();
    present(&mut s, &mut grants).unwrap();
    let (issuer, x, list) = (s.issuer.clone(), s.offer_x.clone(), s.list.clone());
    let l = list.clone();
    s.w.party(&issuer)
        .revoke(&x, |st| l.lock().unwrap().revoke(st.index))
        .unwrap();
    publish(&mut s.w, &issuer, &list);
    // the held grant is dropped at the next status check
    assert_eq!(grants.refresh(&s.w.status, s.w.clock.now_ms()), 1);
    let refusal = admit(
        &policy(&s.target, &grants),
        &opened(&s.agent, "order.placed"),
        &Untyped,
        Timestamp(s.w.clock.now_ms()),
    );
    assert!(
        refusal.is_err(),
        "the agent's events are refused as ungranted"
    );
    // and presenting it again yields no grant
    let mut fresh = PresentedGrants::new();
    assert!(present(&mut s, &mut fresh).is_none());
    assert!(fresh.grant_set().grants().is_empty());
    lean_accepts(&s.w);
}
