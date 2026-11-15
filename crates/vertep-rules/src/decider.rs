//! The rules decider over tiramemsu's ELPI (feature `native`).

use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tiramemsu::{
    ElpiOptions, ElpiProgram, Error as TmError, TimeRef, TxElpi, TxOptions, Valid, Value as TmValue,
};
use vertep_agent::{Annotations, Decider, DeciderKind, Decision, DecisionOutcome, DecisionRequest};
use vertep_core::datamodel::Value;
use vertep_tiramemsu::TiramemsuShard;

use crate::vocab;
use crate::{Derivation, RulePolicy, RulesError};

fn iri(s: impl Into<String>) -> TmValue {
    TmValue::iri(s)
}

fn vt(s: &str) -> TmValue {
    iri(s)
}

fn event_iri(event: &str) -> String {
    format!("{}{event}", vocab::EVENT_PREFIX)
}

fn integer(n: i64) -> TmValue {
    TmValue::literal(
        &n.to_string(),
        Some("http://www.w3.org/2001/XMLSchema#integer"),
        None,
    )
}

/// A scalar context value as a statement object; arrays, objects and absent values are skipped,
/// so a rule can only read what is plainly data.
fn scalar(v: &Value) -> Option<TmValue> {
    match v {
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => Some(integer(*n as i64)),
        Value::Number(n) => Some(TmValue::literal(
            &n.to_string(),
            Some("http://www.w3.org/2001/XMLSchema#double"),
            None,
        )),
        Value::String(s) => Some(TmValue::str(s.clone())),
        Value::Bool(b) => Some(TmValue::literal(
            if *b { "true" } else { "false" },
            Some("http://www.w3.org/2001/XMLSchema#boolean"),
            None,
        )),
        _ => None,
    }
}

/// Stores `text` as the rule program `program` in the shard's file, superseding any earlier
/// source ([[openspec:rule-programs#Rule Programs Live In The Shard]]).
// @tg: implements:: [[openspec:rule-programs#Rule Programs Live In The Shard]]
pub fn put_program(shard: &TiramemsuShard, program: &str, text: &str) -> Result<(), RulesError> {
    let node = iri(program);
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            t.store_elpi_program(node.clone(), text)?;
            Ok(())
        })
        .map(|_| ())
        .map_err(|e| RulesError::Program(e.to_string()))
}

/// The digest of a program closure as `view` sees it.
fn closure_digest(view: &tiramemsu::View<'_>, program: &TmValue) -> Result<String, RulesError> {
    let info = view
        .elpi_program_info(program)
        .map_err(|e| RulesError::Program(e.to_string()))?;
    let mut h = Sha256::new();
    h.update(format!("{program:?}\0{}\0{}\n", info.statement, info.text));
    let mut inc: Vec<String> = info
        .includes
        .iter()
        .map(|(p, e)| format!("{p:?}\0{e}"))
        .collect();
    inc.sort();
    for i in inc {
        h.update(i);
        h.update("\n");
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A decider that runs a stored rule program over the offer
/// ([[openspec:rule-decider#Rule Decider]]).
pub struct RuleDecider {
    shard: Arc<TiramemsuShard>,
    program: String,
    policy: RulePolicy,
    log: Mutex<Vec<Derivation>>,
}

impl RuleDecider {
    /// A decider for the program `program` in `shard`. The program must already be stored with
    /// [`put_program`].
    // @tg: implements:: [[openspec:rule-decider#Rule Decider]] {confidence: partial}
    pub fn new(
        shard: Arc<TiramemsuShard>,
        program: &str,
        policy: RulePolicy,
    ) -> Result<Self, RulesError> {
        let node = iri(program);
        shard
            .db()
            .now()
            .elpi_program_info(&node)
            .map_err(|e| RulesError::Program(e.to_string()))?;
        Ok(RuleDecider {
            shard,
            program: program.to_owned(),
            policy,
            log: Mutex::new(Vec::new()),
        })
    }

    /// The derivations of the decisions made so far, oldest first. The caller records them on
    /// the decision entries (milestone M12 gives them a field).
    pub fn derivations(&self) -> Vec<Derivation> {
        self.log.lock().expect("derivation log").clone()
    }

    // @tg: implements:: [[openspec:rule-decider#Offer And State Are Statements]]
    // @tg: implements:: [[openspec:rule-decider#Derivation Is The Rationale]] {confidence: partial, note: "the decision entry field is M12"}
    // @tg: implements:: [[openspec:rule-decider#No Applicable Rule Is A Refusal]]
    // @tg: implements:: [[openspec:rule-programs#Evaluation Is Bounded]]
    fn run(
        &self,
        request: &DecisionRequest,
    ) -> Result<(Decision, Derivation), Box<DecisionOutcome>> {
        let refuse = |reason: &str| Box::new(DecisionOutcome::refuse(reason.to_owned()));
        let node_iri = format!("{}{}", vocab::NODE_PREFIX, request.correlation);
        let node = iri(node_iri.clone());
        let ctx = vt(vocab::CTX);
        let program = iri(self.program.clone());

        // The offer, the actor, the state and the plain context fields become statements in
        // one transaction, with the context statement that scopes every rule to this decision
        // ([[openspec:rule-decider#Offer And State Are Statements]]).
        let mut ctx_eid = None;
        let report = self
            .shard
            .db()
            .transact(TxOptions::default(), |t| {
                t.assert(
                    node.clone(),
                    vt(vocab::ACTOR),
                    TmValue::str(request.actor.clone()),
                    Valid::ALWAYS,
                )?;
                t.assert(
                    node.clone(),
                    vt(vocab::STATE),
                    TmValue::str(request.state.clone()),
                    Valid::ALWAYS,
                )?;
                for c in &request.candidates {
                    t.assert(
                        node.clone(),
                        vt(vocab::OFFERED),
                        iri(event_iri(&c.event)),
                        Valid::ALWAYS,
                    )?;
                }
                if let Value::Object(fields) = request.context.data() {
                    for (name, value) in fields {
                        if let Some(o) = scalar(value) {
                            t.assert(
                                node.clone(),
                                iri(format!("{}{name}", vocab::CTX_FIELD_PREFIX)),
                                o,
                                Valid::ALWAYS,
                            )?;
                        }
                    }
                }
                ctx_eid = Some(
                    t.assert(
                        ctx.clone(),
                        vt(vocab::DECIDING),
                        node.clone(),
                        Valid::ALWAYS,
                    )?
                    .eid(),
                );
                Ok(())
            })
            .map_err(|e| refuse(&format!("rules-store: {e}")))?;
        let tx = report.t.0;

        let opts = ElpiOptions {
            fuel: self.policy.fuel,
            ..ElpiOptions::default()
        };
        let view = self.shard.db().as_of(TimeRef::Tx(tx));
        let digest = closure_digest(&view, &program).map_err(|e| refuse(&e.to_string()))?;
        let result = view.elpi_conclude(&ElpiProgram::Stored(program.clone()), &opts);

        // The context statement only scopes one run; end it whatever the outcome.
        let _ = self.shard.db().transact(TxOptions::default(), |t| {
            if let Some(e) = ctx_eid {
                t.retract(e)?;
            }
            Ok(())
        });

        let result = match result {
            Ok(r) => r,
            Err(TmError::ElpiOutOfFuel { .. }) => {
                return Err(refuse(
                    &RulesError::FuelExhausted {
                        limit: self.policy.fuel,
                    }
                    .to_string(),
                ));
            }
            Err(e) => return Err(refuse(&format!("rules-error: {e}"))),
        };

        // Conclusions about this node, as event names, with their support.
        let mine: Vec<(String, Vec<String>)> = result
            .conclusions
            .iter()
            .filter(|c| c.s == node && c.p == vt(vocab::DECISION))
            .filter_map(|c| match &c.o {
                TmValue::Iri(o) => o.strip_prefix(vocab::EVENT_PREFIX).map(|e| {
                    (
                        e.to_owned(),
                        c.support.iter().map(|s| s.to_string()).collect(),
                    )
                }),
                _ => None,
            })
            .collect();
        if mine.is_empty() {
            return Err(refuse("no-applicable-rule"));
        }
        // The first offered candidate, in offer order, that the program concluded.
        let chosen = request
            .candidates
            .iter()
            .find_map(|c| mine.iter().find(|(e, _)| *e == c.event));
        let Some((event, support)) = chosen else {
            return Err(refuse("not-offered"));
        };

        let d = Derivation {
            node: node_iri,
            actor: request.actor.clone(),
            state: request.state.clone(),
            program: self.program.clone(),
            event: event.clone(),
            support: support.clone(),
            closure_digest: digest,
            engine: self.policy.engine.clone(),
            fuel_limit: self.policy.fuel,
            fuel_used: result.cost.fuel_used,
            tx,
        };
        let mut decision = Decision::new(event.clone(), Value::Undefined);
        decision.annotations = Annotations {
            rationale: Some(d.summary()),
            decider_kind: Some(DeciderKind::Rules.as_str().to_owned()),
            decider: Some(request.binding.clone()),
            ..Annotations::default()
        };
        Ok((decision, d))
    }
}

impl Decider for RuleDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Rules
    }

    fn decide(
        &self,
        request: DecisionRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = DecisionOutcome> + Send>> {
        // The run is synchronous, bounded by fuel, and leaves nothing running.
        let outcome = match self.run(&request) {
            Ok((decision, derivation)) => {
                self.log.lock().expect("derivation log").push(derivation);
                DecisionOutcome::Decision(decision)
            }
            Err(refusal) => *refusal,
        };
        Box::pin(async move { outcome })
    }
}

/// Re-runs a recorded derivation against the store as of its transaction and checks that it
/// reaches the same event and support under the same closure and engine
/// ([[openspec:rule-programs#Rule Version Is Pinned By The Entry]]).
// @tg: implements:: [[openspec:rule-programs#Rule Version Is Pinned By The Entry]]
pub fn verify(
    shard: &TiramemsuShard,
    d: &Derivation,
    policy: &RulePolicy,
) -> Result<(), RulesError> {
    if d.engine != policy.engine {
        return Err(RulesError::EngineMismatch {
            recorded: d.engine.clone(),
            running: policy.engine.clone(),
        });
    }
    let program = iri(d.program.clone());
    let view = shard.db().as_of(TimeRef::Tx(d.tx));
    let digest = closure_digest(&view, &program)?;
    if digest != d.closure_digest {
        return Err(RulesError::Diverged(format!(
            "closure digest {digest} differs from recorded {}",
            d.closure_digest
        )));
    }
    let opts = ElpiOptions {
        fuel: d.fuel_limit,
        ..ElpiOptions::default()
    };
    let r = view
        .elpi_conclude(&ElpiProgram::Stored(program), &opts)
        .map_err(|e| match e {
            TmError::ElpiOutOfFuel { .. } => RulesError::FuelExhausted {
                limit: d.fuel_limit,
            },
            e => RulesError::Program(e.to_string()),
        })?;
    let node = iri(d.node.clone());
    let want = iri(event_iri(&d.event));
    let found = r
        .conclusions
        .iter()
        .find(|c| c.s == node && c.p == vt(vocab::DECISION) && c.o == want);
    let Some(c) = found else {
        return Err(RulesError::Diverged(format!(
            "no conclusion {} for {}",
            d.event, d.node
        )));
    };
    let support: Vec<String> = c.support.iter().map(|s| s.to_string()).collect();
    if support != d.support {
        return Err(RulesError::Diverged(format!(
            "support {support:?} differs from recorded {:?}",
            d.support
        )));
    }
    Ok(())
}
