//! The credit-decision policy as a stored rule program: three applications decided by rules,
//! the derivation of each, a bundle that verifies with no shard, and the next policy version
//! tried on the recorded decisions.
//!
//! Needs the native engine library:
//! `LEANELPI_DIST=... DYLD_LIBRARY_PATH=... cargo run -p vertep-rules --features native --example credit`

use std::sync::Arc;

use vertep_agent::{Candidate, ContextView, Decider, DecisionOutcome, DecisionRequest};
use vertep_core::datamodel::Value;
use vertep_rules::{RuleBundle, RuleDecider, RulePolicy, counterfactual, put_program};
use vertep_tiramemsu::TiramemsuShard;

const PROGRAM: &str = "urn:vt:program:loan-policy";

/// Approve inside the limits; refer above the ceiling; the ratio rule keeps `approve` out of
/// reach however the amount looks. Every rule starts from the context statement.
const POLICY: &str = r#"
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, Eo, Ed, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:approve") Eo,
  tm.triple C (iri "urn:vt:ctx:dti") (num D) Ed, D =< 43,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A =< 50000.

tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:refer")) [X, Eo, Ea] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:offered") (iri "urn:vt:event:refer") Eo,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) Ea, A > 50000.
"#;

const APPLICATIONS: [(&str, f64, f64, &[&str]); 3] = [
    ("1001", 20_000.0, 31.0, &["approve", "decline", "refer"]),
    ("1002", 80_000.0, 38.0, &["decline", "refer"]),
    ("1003", 45_000.0, 40.0, &["approve", "refer"]),
];

fn candidate(event: &str) -> Candidate {
    Candidate {
        event: event.to_owned(),
        schema: serde_json::json!({}),
        schema_hash: String::new(),
        decision_guard: None,
        source: "underwriting".to_owned(),
        index: 0,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("vertep-rules-credit-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let shard = Arc::new(TiramemsuShard::open(dir.join("shard.db"))?);
    put_program(&shard, PROGRAM, POLICY)?;
    let decider = RuleDecider::new(shard.clone(), PROGRAM, RulePolicy::default())?;

    println!("deciding three applications by rule:");
    for (id, amount, dti, offered) in APPLICATIONS {
        let request = DecisionRequest {
            state: "underwriting".to_owned(),
            candidates: offered.iter().map(|e| candidate(e)).collect(),
            context: ContextView::new(Value::Object(vec![
                ("amount".to_owned(), Value::Number(amount)),
                ("dti".to_owned(), Value::Number(dti)),
            ])),
            actor: format!("https://example.org/loans/{id}"),
            correlation: id.to_owned(),
            binding: "underwriter".to_owned(),
            fallback: false,
        };
        let verdict = match futures_lite(decider.decide(request)) {
            DecisionOutcome::Decision(d) => d.annotations.rationale.unwrap_or(d.event),
            DecisionOutcome::Refusal(r) => format!("refused: {}", r.reason),
        };
        println!("  {id}  amount {amount}, ratio {dti}%, offered {offered:?}\n       {verdict}");
    }

    let recorded = decider.derivations();
    println!("\nthe bundle of the first decision, verified with no shard:");
    let json = RuleBundle::export(&shard, &recorded[0])?.to_json();
    let bundle = RuleBundle::from_json(&json)?;
    bundle.verify(&RulePolicy::default())?;
    println!(
        "  {} bytes; {} concluded {} from {} statements: verified",
        json.len(),
        bundle.derivation.node,
        bundle.derivation.event,
        bundle.derivation.support.len()
    );

    println!("\nthe next policy (ceiling 30 000) tried on the recorded decisions:");
    let next = POLICY
        .replace("A =< 50000", "A =< 30000")
        .replace("A > 50000", "A > 30000");
    let report = counterfactual(&shard, &recorded, &next, &RulePolicy::default())?;
    println!(
        "  {} unchanged, {} differ",
        report.unchanged,
        report.differs.len()
    );
    for d in &report.differs {
        println!(
            "  {}: {} -> {}",
            d.node,
            d.recorded_event,
            d.counterfactual_event.as_deref().unwrap_or("(none)")
        );
    }
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// The decider's future is ready on first poll (the run is synchronous), so a bare poll is
/// enough and the example needs no executor.
fn futures_lite<T>(f: impl std::future::Future<Output = T>) -> T {
    let mut f = std::pin::pin!(f);
    let waker = std::task::Waker::noop();
    match f.as_mut().poll(&mut std::task::Context::from_waker(waker)) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => unreachable!("a rule decision is synchronous"),
    }
}
