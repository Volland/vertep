//! Consent (`agent-wallet` requirement `Consent Before Disclosure`, design D7).
//!
//! A [`ConsentPolicy`] computes the disclosures allowed for a request: claims that are
//! requested, allowed by a rule for that verifier, and held in a live credential. The holder's
//! role chart offers them in its consent decide state; any decider (this module's
//! [`PolicyDecider`], a human prompt, an LLM) picks one or refuses, and a choice outside the
//! offered set is never taken. The decider sees claim names, the verifier and the purpose,
//! never claim values.

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use vertep_agent::decider::{
    Decider, DeciderKind, Decision, DecisionOutcome, DecisionRequest,
};
use vertep_core::datamodel::Value;

use crate::history::Claims;
use crate::plugin::{ConsentPolicy, ConsentRequest, DisclosureOption};

/// One rule: claims a verifier may receive, from credentials of some types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The verifier DID, or `*` for any verifier.
    pub verifier: String,
    /// Claim names it may receive.
    pub claims: Claims,
    /// Credential types the rule covers (empty: any).
    #[serde(default)]
    pub types: Vec<String>,
}

/// A consent policy made of rules; with no rules it refuses everything (the default).
// @tg: implements:: [[openspec:agent-wallet#Consent Before Disclosure]] {confidence: partial, note: "the candidates; the decide state is in the holder role chart"}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulePolicy {
    /// The rules.
    pub rules: Vec<Rule>,
}

impl RulePolicy {
    /// The policy that refuses every request.
    pub fn refuse_all() -> Self {
        RulePolicy::default()
    }

    /// This policy with one more rule.
    #[must_use]
    pub fn allow(mut self, verifier: &str, claims: &[&str]) -> Self {
        self.rules.push(Rule {
            verifier: verifier.to_owned(),
            claims: claims.iter().map(|c| (*c).to_owned()).collect(),
            types: Vec::new(),
        });
        self
    }
}

impl ConsentPolicy for RulePolicy {
    fn options(&self, request: &ConsentRequest) -> Vec<DisclosureOption> {
        let mut out = Vec::new();
        for c in &request.held {
            if !request.types.is_empty() && !c.types.iter().any(|t| request.types.contains(t)) {
                continue;
            }
            let mut allowed = Claims::new();
            for r in &self.rules {
                let verifier_ok = r.verifier == "*" || r.verifier == request.verifier;
                let types_ok = r.types.is_empty() || c.types.iter().any(|t| r.types.contains(t));
                if verifier_ok && types_ok {
                    allowed.extend(r.claims.iter().cloned());
                }
            }
            let within: Claims = allowed
                .intersection(&request.requested)
                .filter(|n| c.claims.contains(*n))
                .cloned()
                .collect();
            if within.is_empty() {
                continue;
            }
            if c.selective {
                out.push(DisclosureOption {
                    credential: c.digest.clone(),
                    claims: within,
                });
            } else if c.claims.iter().all(|n| within.contains(n)) {
                // a non-selective credential discloses all its claims: only when all are allowed
                out.push(DisclosureOption {
                    credential: c.digest.clone(),
                    claims: c.claims.clone(),
                });
            }
        }
        out.sort();
        out
    }
}

/// The event a consent decision chooses to disclose; its payload is `{credential, claims}`.
pub const DISCLOSE_EVENT: &str = "consent.disclose";
/// The event a consent decision chooses to refuse.
pub const REFUSE_EVENT: &str = "consent.refuse";

/// The rule decider: discloses the first offered option in full, or refuses when none is
/// offered.
#[derive(Clone, Copy, Debug, Default)]
pub struct PolicyDecider;

/// The disclosure options a consent offer carries in its context (`options`).
pub fn offered_options(request: &DecisionRequest) -> Vec<DisclosureOption> {
    request
        .context
        .get("options")
        .map(vertep_agent::json::to_json)
        .and_then(|j| serde_json::from_value(j).ok())
        .unwrap_or_default()
}

/// The decision disclosing `option`.
pub fn disclose(option: &DisclosureOption) -> DecisionOutcome {
    let payload = vertep_agent::json::from_json(&serde_json::json!({
        "credential": option.credential,
        "claims": option.claims,
    }));
    DecisionOutcome::Decision(Decision::new(DISCLOSE_EVENT, payload))
}

/// The decision refusing.
pub fn refuse() -> DecisionOutcome {
    DecisionOutcome::Decision(Decision::new(REFUSE_EVENT, Value::Object(Vec::new())))
}

impl Decider for PolicyDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Guard
    }

    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        let outcome = match offered_options(&request).first() {
            Some(o) => disclose(o),
            None => refuse(),
        };
        Box::pin(async move { outcome })
    }
}
