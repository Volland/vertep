//! Totality: does a program answer every offer of a decide state
//! ([[openspec:rule-verification#Totality Check]])? Answered by running the rule decider on every
//! non-empty subset of the candidate events under each sample context, in a scratch store.

use std::sync::Arc;

use vertep_agent::{Candidate, ContextView, Decider, DecisionOutcome, DecisionRequest};
use vertep_core::datamodel::Value;
use vertep_tiramemsu::TiramemsuShard;

use crate::{RuleDecider, RulePolicy, RulesError, Totality, put_program};

/// The largest candidate set the check enumerates (2^n subsets per context).
pub const MAX_CANDIDATES: usize = 8;

fn candidate(event: &str) -> Candidate {
    Candidate {
        event: event.to_owned(),
        schema: serde_json::json!({}),
        schema_hash: String::new(),
        decision_guard: None,
        source: "totality".to_owned(),
        index: 0,
    }
}

fn poll<T>(f: impl std::future::Future<Output = T>) -> T {
    let mut f = std::pin::pin!(f);
    let waker = std::task::Waker::noop();
    match f.as_mut().poll(&mut std::task::Context::from_waker(waker)) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => unreachable!("a rule decision is synchronous"),
    }
}

/// Checks `text` against every non-empty subset of `candidates` under each context in `contexts`.
/// The answer holds for those contexts only: the check does not reason about all values.
// @tg: implements:: [[openspec:rule-verification#Totality Check]]
pub fn totality(
    text: &str,
    candidates: &[&str],
    contexts: &[Vec<(String, Value)>],
    policy: &RulePolicy,
) -> Result<Totality, RulesError> {
    if candidates.is_empty() || contexts.is_empty() {
        return Ok(Totality::Unknown(
            "no candidates or no sample contexts".to_owned(),
        ));
    }
    if candidates.len() > MAX_CANDIDATES {
        return Ok(Totality::Unknown(format!(
            "{} candidates exceed the bound of {MAX_CANDIDATES}",
            candidates.len()
        )));
    }
    let dir = tempfile::tempdir().map_err(|e| RulesError::Store(e.to_string()))?;
    let shard = Arc::new(
        TiramemsuShard::open(dir.path().join("totality.db"))
            .map_err(|e| RulesError::Store(e.to_string()))?,
    );
    const PROGRAM: &str = "urn:vt:program:totality";
    put_program(&shard, PROGRAM, text)?;
    let decider = RuleDecider::new(shard, PROGRAM, policy.clone())?;
    let mut checked = 0;
    for ctx in contexts {
        for mask in 1u32..(1 << candidates.len()) {
            let offer: Vec<&str> = candidates
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, c)| *c)
                .collect();
            let request = DecisionRequest {
                state: "totality".to_owned(),
                candidates: offer.iter().map(|e| candidate(e)).collect(),
                context: ContextView::new(Value::Object(ctx.clone())),
                actor: "urn:vt:totality".to_owned(),
                correlation: format!("t{checked}"),
                binding: "totality".to_owned(),
                fallback: false,
            };
            checked += 1;
            match poll(decider.decide(request)) {
                DecisionOutcome::Decision(_) => {}
                DecisionOutcome::Refusal(r) if r.reason == "no-applicable-rule" => {
                    return Ok(Totality::Gap {
                        offer: offer.iter().map(|s| (*s).to_owned()).collect(),
                        context: ctx.clone(),
                    });
                }
                // an explicit refusal, a not-offered conclusion, or fuel: the program answered
                // (or failed loudly), so it is not a silent gap
                DecisionOutcome::Refusal(r) if r.reason == "explicit-refusal" => {}
                DecisionOutcome::Refusal(r) => {
                    return Ok(Totality::Unknown(format!(
                        "a run refused with {}",
                        r.reason
                    )));
                }
            }
        }
    }
    Ok(Totality::Total { checked })
}
