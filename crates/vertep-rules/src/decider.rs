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
use crate::{DERIVED_GRAPH_SUFFIX, Derivation, RulePolicy, RulesError};

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

pub(crate) fn iri_text(v: &TmValue) -> String {
    match v {
        TmValue::Iri(s) => s.clone(),
        other => format!("{other:?}"),
    }
}

/// The IRI and source text of a program and of each program it includes, sorted by IRI, as
/// `view` sees them.
pub(crate) fn closure_sources(
    view: &tiramemsu::View<'_>,
    program: &TmValue,
) -> Result<Vec<(String, String)>, RulesError> {
    let info = view
        .elpi_program_info(program)
        .map_err(|e| RulesError::Program(e.to_string()))?;
    let mut out = vec![(iri_text(program), info.text.clone())];
    for (p, _) in &info.includes {
        let i = view
            .elpi_program_info(p)
            .map_err(|e| RulesError::Program(e.to_string()))?;
        out.push((iri_text(p), i.text));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The digest of a program closure: it depends on the text only, so it holds in any store.
pub(crate) fn digest_of(sources: &[(String, String)]) -> String {
    let mut h = Sha256::new();
    for (iri, text) in sources {
        h.update(iri.as_bytes());
        h.update([0]);
        h.update(text.as_bytes());
        h.update([b'\n']);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn closure_digest(view: &tiramemsu::View<'_>, program: &TmValue) -> Result<String, RulesError> {
    Ok(digest_of(&closure_sources(view, program)?))
}

/// Turns enforcement of a stored rule program on or off. Turning it on runs the program's full
/// `tm.violation` check over the shard's current data and is refused, with the violations
/// listed, if live data violates it; later commits run its `tm.violation-delta` check and refuse
/// a violating transaction ([[openspec:rule-invariants#Enabling Checks Existing Data]]).
// @tg: implements:: [[openspec:rule-invariants#Enabling Checks Existing Data]]
// @tg: implements:: [[openspec:rule-invariants#Delta Check Agrees With Full Check]] {confidence: partial, note: "the checks are tiramemsu's; this crate proves them equal by a property test"}
// @tg: implements:: [[openspec:rule-invariants#Enforced Program Refuses A Violating Append]] {confidence: partial, note: "the runtime dead-letter path is not built"}
pub fn enforce(shard: &TiramemsuShard, program: &str, on: bool) -> Result<(), RulesError> {
    let node = iri(program);
    shard
        .db()
        .transact(TxOptions::default(), |t| {
            t.enforce_elpi(node.clone(), on)?;
            Ok(())
        })
        .map(|_| ())
        .map_err(|e| match vertep_tiramemsu::violations(&e) {
            Some(v) => RulesError::Violations(v),
            None => RulesError::Program(e.to_string()),
        })
}

/// What a materialization run did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Materialized {
    /// Statements derived by this run, as the store names them.
    pub derived: Vec<String>,
    /// Conclusions that were already derived from the same support.
    pub existing: usize,
    /// Earlier conclusions this run no longer derives, retracted.
    pub retracted: Vec<String>,
    /// True when the program may be non-monotonic: a later write can invalidate its
    /// conclusions before the next run ([[decisions#Derived facts live in the actor's graph with their cause]]).
    pub non_monotonic: bool,
}

/// Runs a stored program's `tm.conclude` conclusions into the store as derived facts, each with
/// the statements it states as support. A fact falls with its support and with the program that
/// concluded it ([[openspec:derived-facts#Derived Facts Carry Support]]).
// @tg: implements:: [[openspec:derived-facts#Derived Facts Carry Support]] {confidence: partial, note: "derived facts are written to the shard, not yet to a named graph per actor"}
// @tg: implements:: [[openspec:derived-facts#Derived Facts Retract With Support]]
pub fn materialize(
    shard: &TiramemsuShard,
    program: &str,
    fuel: u64,
) -> Result<Materialized, RulesError> {
    let opts = tiramemsu::MaterializeElpiOptions {
        elpi: ElpiOptions {
            fuel,
            ..ElpiOptions::default()
        },
        dry_run: false,
    };
    let r = shard
        .db()
        .materialize_elpi(&iri(program), &opts)
        .map_err(|e| match e {
            TmError::ElpiOutOfFuel { .. } => RulesError::FuelExhausted { limit: fuel },
            e => RulesError::Program(e.to_string()),
        })?;
    place_in_actor_graphs(
        shard,
        r.derived.iter().chain(r.existing.iter()).copied().collect(),
    )?;
    Ok(Materialized {
        derived: r.derived.iter().map(ToString::to_string).collect(),
        existing: r.existing.len(),
        retracted: r.reconciled.iter().map(ToString::to_string).collect(),
        non_monotonic: !r.needs_rerun_after_writes.is_empty(),
    })
}

/// Adds each derived statement to the derived-facts graph of its subject, when the subject is an
/// IRI ([[openspec:derived-facts#Derived Facts Carry Support]]).
fn place_in_actor_graphs(
    shard: &TiramemsuShard,
    ids: std::collections::BTreeSet<tiramemsu::Eid>,
) -> Result<(), RulesError> {
    if ids.is_empty() {
        return Ok(());
    }
    let view = shard.db().now();
    let mut place = Vec::new();
    for t in view
        .triples(None, None, None)
        .map_err(|e| RulesError::Store(e.to_string()))?
    {
        if ids.contains(&t.eid)
            && let TmValue::Iri(subject) = view
                .decode(t.s)
                .map_err(|e| RulesError::Store(e.to_string()))?
        {
            place.push((t.eid, format!("{subject}{DERIVED_GRAPH_SUFFIX}")));
        }
    }
    shard
        .db()
        .transact(TxOptions::default(), |tx| {
            for (eid, graph) in &place {
                tx.add_to_graph(
                    *eid,
                    TmValue::iri(graph.clone()),
                    tiramemsu::AssertOpts {
                        valid: Valid::ALWAYS,
                        on_existing: tiramemsu::OnExisting::Return,
                    },
                )?;
            }
            Ok(())
        })
        .map(|_| ())
        .map_err(|e| RulesError::Store(e.to_string()))
}

/// The source text of a stored program, now or as of transaction `tx`.
pub fn program_text(
    shard: &TiramemsuShard,
    program: &str,
    tx: Option<u64>,
) -> Result<String, RulesError> {
    let node = iri(program);
    let view = match tx {
        Some(t) => shard.db().as_of(TimeRef::Tx(t)),
        None => shard.db().now(),
    };
    view.elpi_program_info(&node)
        .map(|i| i.text)
        .map_err(|e| RulesError::Program(e.to_string()))
}

/// What a trial run of a stored program found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckReport {
    /// Conclusions over the current data.
    pub conclusions: usize,
    /// Steps used.
    pub fuel_used: u64,
    /// True when the program may be non-monotonic.
    pub non_monotonic: bool,
}

/// Loads a stored program and runs it once over the current data under `fuel`, so that a
/// syntax or type error, or a runaway rule, shows before the program decides anything.
pub fn check_program(
    shard: &TiramemsuShard,
    program: &str,
    fuel: u64,
) -> Result<CheckReport, RulesError> {
    let opts = ElpiOptions {
        fuel,
        ..ElpiOptions::default()
    };
    shard
        .db()
        .now()
        .elpi_conclude(&ElpiProgram::Stored(iri(program)), &opts)
        .map(|r| CheckReport {
            conclusions: r.conclusions.len(),
            fuel_used: r.cost.fuel_used,
            non_monotonic: r.non_monotonic,
        })
        .map_err(|e| match e {
            TmError::ElpiOutOfFuel { .. } => RulesError::FuelExhausted { limit: fuel },
            e => RulesError::Program(e.to_string()),
        })
}

/// True when every program of the closure is inside the rules profile.
fn profile_checked(view: &tiramemsu::View<'_>, program: &TmValue) -> bool {
    closure_sources(view, program)
        .map(|srcs| srcs.iter().all(|(_, t)| crate::check_profile(t).is_empty()))
        .unwrap_or(false)
}

/// A decider that runs a stored rule program over the offer
/// ([[openspec:rule-decider#Rule Decider]]).
pub struct RuleDecider {
    shard: Arc<TiramemsuShard>,
    program: String,
    scope: Option<String>,
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
            scope: None,
            policy,
            log: Mutex::new(Vec::new()),
        })
    }

    /// Binds the decider to a scope: while a proposal of this program is `Scoped` for the scope, it
    /// runs that proposal instead of the live program.
    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// The program to run now: the proposal scoped to this decider's scope, else the live program.
    fn effective_program(&self) -> String {
        self.scope
            .as_deref()
            .and_then(|s| crate::authoring::scoped_program(&self.shard, &self.program, s))
            .unwrap_or_else(|| self.program.clone())
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
        let program_name = self.effective_program();
        let program = iri(program_name.clone());

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
            // a program may refuse on purpose by concluding `refuse`
            if mine.iter().any(|(e, _)| e == vocab::REFUSE) {
                return Err(refuse("explicit-refusal"));
            }
            return Err(refuse("not-offered"));
        };

        let d = Derivation {
            node: node_iri,
            actor: request.actor.clone(),
            state: request.state.clone(),
            program: program_name,
            event: event.clone(),
            support: support.clone(),
            closure_digest: digest,
            engine: self.policy.engine.clone(),
            fuel_limit: self.policy.fuel,
            fuel_used: result.cost.fuel_used,
            tx,
            profile_checked: profile_checked(&view, &program),
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
