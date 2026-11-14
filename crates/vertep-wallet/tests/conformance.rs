//! Plugin conformance: protocol plugins run under seeded adversarial schedules, and the Lean
//! checker `vertep-wallet` judges every recorded history.

mod common;

use std::sync::Arc;

use common::*;
use vertep_wallet::consent::RulePolicy;
use vertep_wallet::message::ExchangeMessage;
use vertep_wallet::plugin::ExchangeProtocol;
use vertep_wallet::protocol::native::VertepExchange;
use vertep_wallet::testkit::{Schedule, check_history};
use vertep_wallet::vocab::{PROTOCOL_VCE, Role};
use vertep_wallet::{Line, WalletError};

/// One issuance and two presentation requests (to two verifiers) over `protocol`, under
/// `schedule`; returns the world.
fn scenario(protocol: &str, schedule: Schedule, tweak: impl Fn(&vertep_wallet::WalletRegistry) + Clone) -> World {
    let mut w = World::new();
    let issuer = w.agent_with(None, tweak.clone());
    let holder = w.agent_with(None, tweak.clone());
    let v1 = w.agent_with(None, tweak.clone());
    let v2 = w.agent_with(None, tweak);
    for v in [&v1, &v2] {
        w.party(v).wallet_mut().trust(&issuer);
    }
    allow(&w, &holder, RulePolicy::refuse_all().allow("*", &["nationality", "birthDate"]));
    w.party(&issuer).offer(protocol, &holder, id_offer()).unwrap();
    w.kit.run(schedule);
    w.party(&v1).request(protocol, &holder, &["nationality"], &[], None).unwrap();
    w.party(&v2).request(protocol, &holder, &["birthDate", "name"], &[], None).unwrap();
    w.kit.run(schedule);
    w
}

fn lean_required() -> bool {
    std::env::var("VERTEP_WALLET_LEAN").as_deref() == Ok("1")
}

// @tg: verifies:: [[openspec:credential-exchange#Plugin Conformance#Built-in plugins conformant]] {level: integration}
#[test]
fn built_in_plugins_conformant() {
    let seeds: u64 = std::env::var("VERTEP_WALLET_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
    let mut checked = 0;
    let mut accepted = 0;
    for seed in 0..seeds {
        for schedule in [Schedule::adversarial(seed), Schedule::lossy(seed)] {
            let w = scenario(PROTOCOL_VCE, schedule, |_| {});
            match w.kit.check() {
                Some(Ok(_)) => checked += 1,
                Some(Err(report)) => panic!("seed {seed}: rejected\n{report}\n{}", w.kit.jsonl()),
                None if lean_required() => panic!("the Lean checker is not built"),
                None => {}
            }
            let accepts = w.history().lines().iter().filter(|l| matches!(l, Line::Verify { result, .. } if result == "accept")).count();
            accepted += accepts;
        }
    }
    if checked == 0 {
        eprintln!("skipped: the Lean checker vertep-wallet is not built");
    }
    assert!(accepted > 0, "no presentation was ever accepted: the schedules starve the exchanges");
}

#[test]
fn honest_run_is_accepted_by_the_checker() {
    let w = scenario(PROTOCOL_VCE, Schedule::honest(), |_| {});
    lean_accepts(&w);
}

const FAULTY: &str = "https://example.org/protocols/faulty-verifier";

/// The native protocol with a faulty verifier chart: after accepting, it keeps waiting and
/// accepts again, so it never treats a nonce as used.
struct FaultyVerifier;

impl ExchangeProtocol for FaultyVerifier {
    fn iri(&self) -> &str {
        FAULTY
    }
    fn role_chart(&self, role: Role) -> &'static str {
        match role {
            Role::Verifier => Box::leak(
                vertep_wallet::exchange::charts::VERIFIER
                    .replace(r#"<transition event="presentation.valid" target="accepted">"#, r#"<transition event="presentation.valid">"#)
                    .into_boxed_str(),
            ),
            other => vertep_wallet::exchange::charts::generic(other),
        }
    }
    fn encode(&self, m: &ExchangeMessage) -> Result<Vec<u8>, WalletError> {
        VertepExchange.encode(m)
    }
    fn decode(&self, b: &[u8]) -> Result<ExchangeMessage, WalletError> {
        VertepExchange.decode(b)
    }
}

// @tg: verifies:: [[openspec:credential-exchange#Plugin Conformance#Faulty plugin caught]] {level: integration}
#[test]
fn faulty_plugin_caught() {
    let register = |r: &vertep_wallet::WalletRegistry| r.register_protocol(Arc::new(FaultyVerifier));
    // duplicates make the faulty verifier accept the same presentation twice
    let schedule = Schedule { duplicate: 100, ..Schedule::honest() };
    let w = scenario(FAULTY, schedule, register);
    let accepts = w.history().lines().iter().filter(|l| matches!(l, Line::Verify { result, .. } if result == "accept")).count();
    assert!(accepts >= 2, "the faulty verifier should accept a duplicate: {}", w.kit.jsonl());
    match check_history(&w.kit.jsonl()) {
        Some(Err(report)) => assert!(report.contains("verify"), "{report}"),
        Some(Ok(report)) => panic!("the checker accepts a faulty plugin's history: {report}"),
        None if lean_required() => panic!("the Lean checker is not built"),
        None => eprintln!("skipped: the Lean checker vertep-wallet is not built"),
    }
}
