//! The candidate set of a decide state: the pure `enabled_events` function and candidate-set
//! fingerprints.
//!
//! `enabled_events` runs SCXML transition selection ([`vertep_core::kernel::select_transitions`])
//! for every event the decide state could take, on the current configuration and history,
//! with guards evaluated by a *probe* datamodel restored from the actor's snapshot, so the
//! actor's own datamodel is never touched. A candidate is an event for which selection would
//! take a transition whose source is the decide state, one of its active descendants or one of
//! its ancestors (an ancestor's transition is what SCXML selection takes when the decide state
//! has none).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use vertep_core::compile::{CompiledChart, StateIdx};
use vertep_core::datamodel::{Datamodel, Event, EventType, StateQuery, Value};
use vertep_core::kernel::{descriptor_matches, is_atomic, proper_ancestors, select_transitions};
use vertep_core::session::Session;

use crate::json::sha256;
use crate::schema::{EventSchemas, schema_hash};

/// One offered candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// The event name a decision may choose.
    pub event: String,
    /// The payload's JSON Schema.
    pub schema: Json,
    /// The schema's content hash.
    pub schema_hash: String,
    /// The `vt:decisionGuard` declared on the enabled transition, for guard deciders.
    pub decision_guard: Option<String>,
    /// The enabled transition at offer time: source state id and index among its transitions.
    pub source: String,
    /// The transition's index in its source's transitions.
    pub index: usize,
}

/// Events never offered: platform and agent events are raised by the runtime, not chosen.
pub fn is_reserved_event(name: &str) -> bool {
    ["error.", "done.", "vt.", "vt:", "cancel.invoke"]
        .iter()
        .any(|p| name.starts_with(p))
}

struct Active<'a> {
    chart: &'a CompiledChart,
    session: &'a Session,
}

impl StateQuery for Active<'_> {
    fn in_state(&self, id: &str) -> bool {
        self.session.is_active(self.chart, id)
    }

    fn active_states(&self) -> Option<Vec<String>> {
        Some(self.session.active(self.chart))
    }
}

fn references_event(cond: &str) -> bool {
    cond.contains("ev.") || cond.contains("_event") || cond.contains("event.")
}

fn related(chart: &CompiledChart, decide: StateIdx, s: StateIdx) -> bool {
    s == decide || chart.is_descendant(s, decide) || chart.is_descendant(decide, s)
}

/// The candidate set of decide state `state`: every event whose transition would be enabled
/// from the current configuration under the current datamodel (ancestors' transitions
/// included), restricted to the `declared` descriptors when given, in document order.
///
/// `probe` is a datamodel already holding the actor's current state (a restored snapshot);
/// its `_event` is overwritten. A guard that fails to evaluate because it reads the event
/// payload, which a decision only supplies later, counts as holding; any other failing guard
/// counts as false.
// @tg: implements:: [[openspec:deciders#Enabled Event Offer]]
pub fn enabled_events(
    chart: &CompiledChart,
    session: &Session,
    probe: &mut dyn Datamodel,
    state: &str,
    declared: Option<&[String]>,
    schemas: &EventSchemas,
) -> Vec<Candidate> {
    let Some(decide) = chart.index_of(state) else {
        return Vec::new();
    };
    if !session.configuration.contains(decide) {
        return Vec::new();
    }
    // Event names reachable from the decide state's chain, in document order.
    let mut names: Vec<String> = Vec::new();
    for atomic in session.configuration.to_vec() {
        if !is_atomic(chart, atomic) || !related(chart, decide, atomic) {
            continue;
        }
        let mut chain = vec![atomic];
        chain.extend(proper_ancestors(chart, atomic, None));
        for s in chain {
            for &t in &chart.state(s).transitions {
                for d in &chart.transition(t).events {
                    let name = d.trim();
                    let name = name.strip_suffix(".*").unwrap_or(name);
                    let name = name.strip_suffix('.').unwrap_or(name);
                    if name.is_empty() || name == "*" || is_reserved_event(name) {
                        continue;
                    }
                    if let Some(decl) = declared
                        && !decl.iter().any(|x| descriptor_matches(x, name))
                    {
                        continue;
                    }
                    if !names.iter().any(|n| n == name) {
                        names.push(name.to_owned());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    for name in names {
        probe.set_event(&Event {
            name: name.clone(),
            type_: EventType::Internal,
            data: Value::Undefined,
            ..Event::default()
        });
        let q = Active { chart, session };
        let selected = select_transitions(
            chart,
            &session.configuration,
            &session.history,
            Some(&name),
            |t| {
                let Some(cond) = chart.transition(t).cond.as_deref() else {
                    return true;
                };
                match probe.eval_cond(cond, &q) {
                    Ok(b) => b,
                    Err(_) => references_event(cond),
                }
            },
        );
        let Some(&t) = selected
            .iter()
            .find(|&&t| related(chart, decide, chart.transition(t).source))
        else {
            continue;
        };
        let tr = chart.transition(t);
        let source = chart.state(tr.source).id.clone();
        let index = tr.index.unwrap_or(0);
        let decision_guard = chart
            .chart()
            .states
            .iter()
            .find_map(|s| find(s, &source))
            .and_then(|s| s.transitions.get(index))
            .and_then(|t| {
                t.extensions.iter().find_map(|e| match &e.value {
                    vertep_core::ir::ExtensionValue::Attribute(v)
                        if e.ns == vertep_core::ir::VT_NS && e.local == "decisionGuard" =>
                    {
                        Some(v.clone())
                    }
                    _ => None,
                })
            });
        let schema = schemas.payload(&name);
        out.push(Candidate {
            schema_hash: schema_hash(&schema),
            schema,
            event: name,
            decision_guard,
            source,
            index,
        });
    }
    out
}

/// The transition SCXML selection would take for `event` (with its real payload) from the
/// decide state `state`'s region, as (source id, index), or `None` when none is enabled.
/// Guards are evaluated by `probe`, whose `_event` is overwritten.
pub fn selected_transition(
    chart: &CompiledChart,
    session: &Session,
    probe: &mut dyn Datamodel,
    state: &str,
    event: &Event,
) -> Option<(String, usize)> {
    let decide = chart.index_of(state)?;
    probe.set_event(event);
    let q = Active { chart, session };
    let selected = select_transitions(
        chart,
        &session.configuration,
        &session.history,
        Some(&event.name),
        |t| {
            chart
                .transition(t)
                .cond
                .as_deref()
                .is_none_or(|c| probe.eval_cond(c, &q).unwrap_or(false))
        },
    );
    selected
        .iter()
        .find(|&&t| related(chart, decide, chart.transition(t).source))
        .map(|&t| {
            let tr = chart.transition(t);
            (chart.state(tr.source).id.clone(), tr.index.unwrap_or(0))
        })
}

fn find<'a>(s: &'a vertep_core::ir::State, id: &str) -> Option<&'a vertep_core::ir::State> {
    if s.id == id {
        return Some(s);
    }
    s.children.iter().find_map(|c| find(c, id))
}

/// The fingerprint of a candidate set: its event names and schema hashes, in order.
// @tg: implements:: [[openspec:deciders#Re-decision After Context Change]]
pub fn fingerprint(candidates: &[Candidate]) -> String {
    let mut text = String::new();
    for c in candidates {
        text.push_str(&c.event);
        text.push('|');
        text.push_str(&c.schema_hash);
        text.push('\n');
    }
    sha256(text.as_bytes())
}
