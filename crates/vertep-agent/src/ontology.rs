//! The `vt-agent:` ontology module, its SHACL shapes and the provenance graph of an agent run.
//!
//! The module and the shapes ship with `vertep-rdf` ([`vertep_rdf::AGENT_ONTOLOGY_TTL`],
//! [`vertep_rdf::AGENT_SHAPES_TTL`]). [`records_graph`] turns an agent machine's
//! [`Record`]s into `vt-agent:` statements: decisions (with RDF 1.2 triple-term annotations on
//! the statement `decision vt-agent:chose event`), model calls with usage, latency and cost,
//! tool calls, plans with their lineage and refused sends. The graph is closed under the
//! module's PROV-O alignment axioms, so a PROV-O query works on it without a reasoner.

use std::sync::OnceLock;

use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Graph, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use vertep_core::ir::VT_NS;
use vertep_rdf::io::{RdfFormat, read_graph};
use vertep_rdf::shacl::{Shapes, ValidationReport};

use crate::decider::Annotations;
use crate::machine::Record;
use crate::vocab;

/// The agent module as a graph: its classes, properties and PROV-O alignment axioms.
// @tg: implements:: [[openspec:llm-actors#Agent Ontology Module]]
pub fn agent_ontology() -> &'static Graph {
    static G: OnceLock<Graph> = OnceLock::new();
    G.get_or_init(|| {
        read_graph(vertep_rdf::AGENT_ONTOLOGY_TTL.as_bytes(), RdfFormat::Turtle)
            .unwrap_or_else(|e| panic!("the bundled agent ontology parses: {e:?}"))
    })
}

/// The compiled agent shapes.
pub fn agent_shapes() -> &'static Shapes {
    static SHAPES: OnceLock<Shapes> = OnceLock::new();
    SHAPES.get_or_init(|| {
        let g = read_graph(vertep_rdf::AGENT_SHAPES_TTL.as_bytes(), RdfFormat::Turtle)
            .unwrap_or_else(|e| panic!("the bundled agent shapes parse: {e:?}"));
        Shapes::from_graph(&g).unwrap_or_else(|e| panic!("the bundled agent shapes compile: {e:?}"))
    })
}

/// Validates agent records against the `vt-agent:` shapes.
// @tg: implements:: [[openspec:llm-actors#Agent Ontology Module]]
pub fn validate(data: &Graph) -> ValidationReport {
    agent_shapes().validate(data)
}

/// Whether `g` uses any `vt-agent:` term (as subject, predicate or object).
pub fn uses_agent_terms(g: &Graph) -> bool {
    let is = |t: &str| t.starts_with(vocab::AGENT_NS);
    g.iter().any(|t| {
        let s = matches!(t.subject, oxrdf::NamedOrBlankNodeRef::NamedNode(n) if is(n.as_str()));
        let o = matches!(t.object, oxrdf::TermRef::NamedNode(n) if is(n.as_str()));
        s || is(t.predicate.as_str()) || o
    })
}

/// `text` as an IRI: itself when it is one (a `vt:` short form expanded), otherwise a
/// `urn:vertep:` IRI with the text percent-encoded.
pub fn iri(text: &str) -> NamedNode {
    let full = match text.strip_prefix("vt:") {
        Some(rest) => format!("{VT_NS}{rest}"),
        None => text.to_owned(),
    };
    NamedNode::new(full)
        .unwrap_or_else(|_| NamedNode::new_unchecked(format!("urn:vertep:{}", encode(text))))
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The IRI of a decider binding.
pub fn decider_iri(binding: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("urn:vertep:decider:{}", encode(binding)))
}

/// The IRI of an event a decision chose: `vt:event/<name>`.
pub fn event_iri(name: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("{VT_NS}event/{}", encode(name)))
}

fn p(term: &str) -> NamedNode {
    NamedNode::new_unchecked(term)
}

fn string(v: &str) -> Term {
    Literal::new_simple_literal(v).into()
}

fn integer(v: impl std::fmt::Display) -> Term {
    Literal::new_typed_literal(v.to_string(), xsd::INTEGER).into()
}

fn decimal(v: f64) -> Term {
    let mut text = format!("{v}");
    if !text.contains('.') {
        text.push_str(".0");
    }
    Literal::new_typed_literal(text, xsd::DECIMAL).into()
}

fn boolean(v: bool) -> Term {
    Literal::new_typed_literal(if v { "true" } else { "false" }, xsd::BOOLEAN).into()
}

struct Out(Graph);

impl Out {
    fn add(&mut self, s: &NamedNode, pred: &str, o: impl Into<Term>) {
        self.0.insert(&Triple::new(s.clone(), p(pred), o.into()));
    }

    fn typed(&mut self, s: &NamedNode, class: &str) {
        self.0.insert(&Triple::new(s.clone(), rdf::TYPE, p(class)));
    }

    fn annotations(&mut self, decision: &NamedNode, event: &str, a: &Annotations) {
        let chose = Triple::new(decision.clone(), p(vocab::CHOSE), event_iri(event));
        let r = NamedNode::new_unchecked(format!("{}/annotation", decision.as_str()));
        let mut any = false;
        if let Some(x) = &a.rationale {
            self.add(&r, vocab::RATIONALE, string(x));
            any = true;
        }
        if let Some(c) = a.confidence {
            self.add(&r, vocab::CONFIDENCE, decimal(c));
            any = true;
        }
        if let Some(m) = &a.model {
            self.add(&r, vocab::MODEL, string(m));
            any = true;
        }
        if let Some(h) = &a.prompt_hash {
            self.add(&r, vocab::PROMPT_HASH, string(h));
            any = true;
        }
        if let Some(k) = &a.decider_kind {
            self.add(&r, vocab::DECIDER_KIND, string(k));
            any = true;
        }
        if a.fallback {
            self.add(&r, vocab::FALLBACK, boolean(true));
            any = true;
        }
        if any {
            self.0.insert(&Triple::new(
                NamedOrBlankNode::from(r),
                rdf::REIFIES,
                Term::Triple(Box::new(chose)),
            ));
        }
    }
}

/// The `vt-agent:` graph of `records`, closed under the module's PROV-O alignment axioms.
// @tg: implements:: [[openspec:deciders#Decision Annotations]]
// @tg: implements:: [[openspec:llm-actors#Usage and Cost Accounting]]
// @tg: implements:: [[openspec:plan-charts#Plan Lineage]]
pub fn records_graph(records: &[Record]) -> Graph {
    let mut g = Out(Graph::new());
    for r in records {
        match r {
            Record::Decision {
                iri: id,
                actor,
                state,
                correlation,
                event,
                binding,
                annotations,
                accepted,
                reason,
            } => {
                let d = iri(id);
                g.typed(&d, vocab::DECISION);
                g.add(&d, vocab::CHOSE, event_iri(event));
                g.add(&d, vocab::CORRELATION_ID, string(correlation));
                g.add(&d, vocab::ACTOR, iri(actor));
                g.add(&d, vocab::ACCEPTED, boolean(*accepted));
                if !state.is_empty() {
                    g.add(&d, vocab::DECIDE_STATE, string(state));
                }
                if let Some(x) = reason {
                    g.add(&d, vocab::REASON, string(x));
                }
                if !binding.is_empty() {
                    let dec = decider_iri(binding);
                    g.typed(&dec, vocab::DECIDER);
                    g.add(&dec, vocab::BINDING, string(binding));
                    g.add(&d, vocab::DECIDER_P, dec);
                }
                g.annotations(&d, event, annotations);
            }
            Record::LlmCall {
                iri: id,
                actor,
                invokeid,
                provider,
                model,
                prompt_hash,
                usage,
                latency_ms,
                cost,
                attempt,
                outcome,
            } => {
                let c = iri(id);
                g.typed(&c, vocab::LLM_CALL);
                g.add(&c, vocab::ACTOR, iri(actor));
                g.add(&c, vocab::INVOKE_ID, string(invokeid));
                g.add(&c, vocab::PROVIDER, string(provider));
                if !model.is_empty() {
                    g.add(&c, vocab::MODEL, string(model));
                }
                g.add(&c, vocab::PROMPT_HASH, string(prompt_hash));
                g.add(&c, vocab::PROMPT_TOKENS, integer(usage.prompt_tokens));
                g.add(
                    &c,
                    vocab::COMPLETION_TOKENS,
                    integer(usage.completion_tokens),
                );
                g.add(&c, vocab::LATENCY_MS, integer(latency_ms.max(&0)));
                g.add(&c, vocab::ATTEMPT, integer(attempt));
                g.add(&c, vocab::OUTCOME, string(outcome));
                if let Some(cost) = cost {
                    g.add(&c, vocab::COST, decimal(cost.amount));
                    g.add(&c, vocab::CURRENCY, string(&cost.currency));
                }
            }
            Record::ToolCall {
                iri: id,
                actor,
                tool,
                outcome,
            } => {
                let c = iri(id);
                let t = iri(tool);
                g.typed(&c, vocab::TOOL_CALL);
                g.typed(&t, vocab::TOOL);
                g.add(&c, vocab::TOOL_P, t);
                g.add(&c, vocab::ACTOR, iri(actor));
                g.add(&c, vocab::OUTCOME, string(outcome));
            }
            Record::Plan {
                iri: id,
                actor,
                state,
                invokeid,
                content_hash,
                generated_by,
                accepted,
                violations,
            } => {
                let pl = iri(id);
                g.typed(&pl, vocab::PLAN);
                g.add(&pl, vocab::CONTENT_HASH, string(content_hash));
                g.add(&pl, vocab::ACCEPTED, boolean(*accepted));
                g.add(&pl, vocab::ACTOR, iri(actor));
                g.add(&pl, vocab::INVOKING_STATE, string(state));
                g.add(&pl, vocab::INVOKE_ID, string(invokeid));
                if let Some(by) = generated_by {
                    g.add(&pl, vocab::GENERATED_BY, iri(by));
                }
                for v in violations {
                    g.add(&pl, vocab::VIOLATION, string(v));
                }
            }
            Record::PlanExecution {
                iri: id,
                plan,
                actor,
                outcome,
            } => {
                let x = iri(id);
                g.typed(&x, vocab::PLAN_EXECUTION);
                g.add(&x, vocab::EXECUTES, iri(plan));
                g.add(&x, vocab::ACTOR, iri(actor));
                g.add(&x, vocab::OUTCOME, string(outcome));
            }
            Record::Denied {
                iri: id,
                sender,
                target,
                event,
            } => {
                let x = iri(id);
                g.typed(&x, vocab::DENIED_SEND);
                g.add(&x, vocab::SENDER, iri(sender));
                g.add(&x, vocab::TARGET, string(target));
                g.add(&x, vocab::EVENT_NAME, string(event));
            }
        }
    }
    vertep_runtime::ontology::rdfs_closure(&g.0, agent_ontology())
}
