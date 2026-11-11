//! Journal entries as RDF: what one turn records and how it is read back.
//!
//! An entry is the named graph named `<address>/entry/<position>`. An input entry records the
//! accepted input (event name, type, payload, send id, origin, origin type, invoke id, message
//! id), the chart version, the timestamp stamped at dequeue, the configuration digest after the
//! macrostep, and its effects: outgoing sends, timers scheduled and cancelled, invocations
//! started, requests issued and answered, transitions taken (annotated with the entry as their
//! cause) and whether the session terminated. Migration and fork entries carry snapshots
//! instead. The store adds `rdf:type vt:JournalEntry`, `vt:actor` and `vt:position`.

use core::fmt;
use std::collections::BTreeSet;

use oxrdf::vocab::xsd;
use oxrdf::{Graph, Literal, NamedNode, NamedNodeRef, Term, TermRef, Triple};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_store::reification::Annotation;
use vertep_store::{Entry, JournalRecord};

use crate::address::{self, Address, hex};
use crate::chart::UnhandledPolicy;
use crate::json;
use crate::vocab::{self as vt, node};

/// A malformed journal entry or snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed journal record: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

/// A reminder registration carried by a timer.
// @tg: implements:: [[openspec:durable-timers#Recurring Reminders]]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderSpec {
    /// The reminder name.
    pub name: String,
    /// The period in milliseconds.
    pub period_ms: i64,
}

/// A durable timer: a delayed send, a request timeout or a reminder.
// @tg: implements:: [[openspec:durable-timers#Delayed Send Is Journaled]]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimerRecord {
    /// The timer IRI, `<entry>/timer/<send id>`; also the message id of its event (for a
    /// reminder, of its occurrences plus `/<n>`).
    pub iri: String,
    /// The actor that scheduled it.
    pub actor: String,
    /// The send id (for a reminder, its name).
    pub send_id: String,
    /// Due time in milliseconds: the scheduling entry's timestamp plus the delay (for a
    /// reminder, of the first occurrence).
    pub due_ms: i64,
    /// The event delivered when it fires.
    pub event: Event,
    /// The address the event is delivered to.
    pub recipient: String,
    /// Set for a reminder.
    pub reminder: Option<ReminderSpec>,
    /// Set for a request timeout: the request's correlation id.
    pub correlation: Option<String>,
}

impl TimerRecord {
    /// The message id of occurrence `n` (0 for a one-shot timer).
    pub fn message_id(&self, n: u64) -> String {
        if self.reminder.is_some() {
            format!("{}/{n}", self.iri)
        } else {
            self.iri.clone()
        }
    }

    /// The due time of occurrence `n` of a reminder (`n` from 0), or of the timer.
    // @tg: implements:: [[openspec:durable-timers#Timer Time Source]]
    pub fn due_of(&self, n: u64) -> i64 {
        match &self.reminder {
            Some(r) => self.due_ms.saturating_add(
                r.period_ms
                    .saturating_mul(i64::try_from(n).unwrap_or(i64::MAX)),
            ),
            None => self.due_ms,
        }
    }
}

/// An outgoing send recorded in an entry; dispatched by the outbox after the append.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendRecord {
    /// The send IRI `<entry>/send/<k>`, which is also the message id it is delivered with.
    pub iri: String,
    /// The send id.
    pub send_id: String,
    /// The event.
    pub event: Event,
    /// The target as the chart gave it.
    pub target: Option<String>,
    /// The resolved recipient address, when the target names one.
    pub recipient: Option<String>,
    /// The Event I/O Processor type.
    pub type_: String,
    /// A request's correlation id, or the correlation id a reply answers.
    pub correlation: Option<String>,
}

/// The accepted input of an entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    /// The event as delivered (origin, origin type, send id, invoke id, payload).
    pub event: Event,
    /// The deduplication key.
    pub message_id: String,
    /// The correlation id the message carried (a request, or a reply).
    pub correlation: Option<String>,
    /// The timer whose firing this input is.
    pub timer: Option<String>,
}

/// A snapshot of an actor at a journal position.
// @tg: implements:: [[openspec:event-journal#Snapshots]]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotData {
    /// The snapshot IRI.
    pub iri: String,
    /// The actor.
    pub actor: String,
    /// The position after which it was taken.
    pub position: u64,
    /// The chart version it is valid under.
    pub version: String,
    /// The configuration digest at that position.
    pub digest: String,
    /// The interpreter session as JSON.
    pub session: String,
    /// The datamodel snapshot as JSON (Vertep value encoding).
    pub datamodel: String,
    /// The runtime state (timers, reminders, requests) as JSON.
    pub runtime: String,
}

/// What an input entry records besides its input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    /// Outgoing sends, in order.
    pub sends: Vec<SendRecord>,
    /// Timers scheduled.
    pub timers: Vec<TimerRecord>,
    /// IRIs of timers cancelled.
    pub cancels: Vec<String>,
    /// Invocations started, as JSON.
    pub invokes: Vec<String>,
    /// Correlation ids of requests issued.
    pub requests: Vec<String>,
    /// The correlation id the input answers.
    pub answers: Option<String>,
    /// Node IRIs of the transitions taken.
    pub transitions: Vec<String>,
    /// Whether the session terminated.
    pub terminated: bool,
    /// The unhandled-event policy applied, when the input enabled no transition.
    pub unhandled: Option<UnhandledPolicy>,
}

/// The kind-specific content of an entry.
#[derive(Clone, Debug, PartialEq)]
pub enum EntryBody {
    /// One accepted input and its macrostep.
    Input {
        /// The input.
        input: InputRecord,
        /// Its effects.
        effects: Effects,
    },
    /// A migration.
    Migration {
        /// The migration IRI.
        migration: String,
        /// The version migrated from.
        from: String,
        /// The state before, under `from`.
        before: SnapshotData,
        /// The state after, under the entry's version.
        after: SnapshotData,
    },
    /// The first entry of a fork.
    Fork {
        /// The source actor.
        source: String,
        /// The source position.
        position: u64,
        /// The source's state there.
        snapshot: SnapshotData,
    },
}

/// One journal entry, decoded.
// @tg: implements:: [[openspec:event-journal#Journal Of Accepted Inputs]]
// @tg: implements:: [[openspec:event-journal#Journal Entry Content]]
// @tg: implements:: [[openspec:chart-versioning#Version Recorded Per Journal Segment]]
#[derive(Clone, Debug, PartialEq)]
pub struct EntryData {
    /// The actor.
    pub actor: Address,
    /// The position.
    pub position: u64,
    /// The chart version IRI the entry ran under.
    pub version: String,
    /// The message id the entry accepted.
    pub message_id: String,
    /// Milliseconds since the epoch, stamped at dequeue.
    pub timestamp_ms: i64,
    /// The configuration digest after the entry.
    pub digest: String,
    /// The kind-specific content.
    pub body: EntryBody,
}

/// The digest of a configuration and datamodel snapshot: SHA-256 (hex) of the active state ids,
/// the datamodel JSON and the termination flag.
pub fn digest(active: &[String], datamodel: &Value, terminated: bool) -> String {
    let mut h = Sha256::new();
    for s in active {
        h.update(s.as_bytes());
        h.update([0u8]);
    }
    h.update([1u8]);
    h.update(json::to_text(datamodel).as_bytes());
    h.update([u8::from(terminated)]);
    hex(&h.finalize())
}

fn int(n: i64) -> Literal {
    Literal::new_typed_literal(n.to_string(), xsd::INTEGER)
}

fn uint(n: u64) -> Literal {
    Literal::new_typed_literal(n.to_string(), xsd::INTEGER)
}

fn boolean(b: bool) -> Literal {
    Literal::new_typed_literal(if b { "true" } else { "false" }, xsd::BOOLEAN)
}

fn str_lit(s: &str) -> Literal {
    Literal::new_simple_literal(s)
}

/// Builds triples about one subject.
struct Out<'a> {
    g: &'a mut Vec<Triple>,
}

impl Out<'_> {
    fn add(&mut self, s: &NamedNode, p: &str, o: impl Into<Term>) {
        self.g.push(Triple::new(s.clone(), node(p), o.into()));
    }

    fn event(&mut self, s: &NamedNode, e: &Event) {
        self.add(s, vt::EVENT_NAME, str_lit(&e.name));
        self.add(s, vt::EVENT_TYPE, str_lit(event_type_name(e.type_)));
        if let Some(x) = &e.sendid {
            self.add(s, vt::SEND_ID, str_lit(x));
        }
        if let Some(x) = &e.origin {
            self.add(s, vt::ORIGIN, str_lit(x));
        }
        if let Some(x) = &e.origintype {
            self.add(s, vt::ORIGIN_TYPE, str_lit(x));
        }
        if let Some(x) = &e.invokeid {
            self.add(s, vt::INVOKE_ID, str_lit(x));
        }
        self.add(s, vt::PAYLOAD, str_lit(&json::to_text(&e.data)));
    }

    fn timer(&mut self, t: &TimerRecord) {
        let s = NamedNode::new_unchecked(&t.iri);
        self.add(
            &s,
            vt::RDF_TYPE,
            node(if t.reminder.is_some() {
                vt::REMINDER
            } else {
                vt::TIMER
            }),
        );
        self.add(&s, vt::ACTOR, NamedNode::new_unchecked(&t.actor));
        self.add(&s, vt::SEND_ID, str_lit(&t.send_id));
        self.add(&s, vt::DUE_AT, int(t.due_ms));
        self.add(&s, vt::RECIPIENT, NamedNode::new_unchecked(&t.recipient));
        self.add(&s, vt::MESSAGE_ID, str_lit(&t.message_id(0)));
        self.add(
            &s,
            vt::ENVELOPE,
            str_lit(&serde_json::to_string(t).unwrap_or_default()),
        );
        if let Some(r) = &t.reminder {
            self.add(&s, vt::REMINDER_NAME, str_lit(&r.name));
            self.add(&s, vt::PERIOD, int(r.period_ms));
        }
        if let Some(c) = &t.correlation {
            self.add(&s, vt::CORRELATION_ID, str_lit(c));
        }
        self.add(&s, vt::EVENT_NAME, str_lit(&t.event.name));
    }

    fn snapshot(&mut self, s: &SnapshotData) {
        let n = NamedNode::new_unchecked(&s.iri);
        self.add(&n, vt::RDF_TYPE, node(vt::SNAPSHOT));
        self.add(&n, vt::SNAPSHOT_OF, NamedNode::new_unchecked(&s.actor));
        self.add(&n, vt::POSITION, uint(s.position));
        self.add(&n, vt::CHART_VERSION, NamedNode::new_unchecked(&s.version));
        self.add(&n, vt::CONFIGURATION_DIGEST, str_lit(&s.digest));
        self.add(&n, vt::SESSION_STATE, str_lit(&s.session));
        self.add(&n, vt::DATAMODEL_STATE, str_lit(&s.datamodel));
        self.add(&n, vt::RUNTIME_STATE, str_lit(&s.runtime));
    }
}

/// The triples of a snapshot record.
pub fn snapshot_triples(s: &SnapshotData) -> Vec<Triple> {
    let mut g = Vec::new();
    Out { g: &mut g }.snapshot(s);
    g
}

/// The triples describing a pending timer in its actor's current-state graph.
pub fn timer_triples(t: &TimerRecord) -> Vec<Triple> {
    let mut g = Vec::new();
    Out { g: &mut g }.timer(t);
    g
}

fn event_type_name(t: EventType) -> &'static str {
    match t {
        EventType::External => "external",
        EventType::Internal => "internal",
        EventType::Platform => "platform",
    }
}

fn event_type_of(s: &str) -> EventType {
    match s {
        "internal" => EventType::Internal,
        "platform" => EventType::Platform,
        _ => EventType::External,
    }
}

impl EntryData {
    /// The entry IRI.
    pub fn iri(&self) -> NamedNode {
        self.actor.entry_iri(self.position)
    }

    /// The effects of an input entry; empty for other kinds.
    pub fn effects(&self) -> Option<&Effects> {
        match &self.body {
            EntryBody::Input { effects, .. } => Some(effects),
            _ => None,
        }
    }

    /// The entry as an RDF graph for [`vertep_store::JournalStore::append`].
    pub fn to_entry(&self) -> Entry {
        let e = self.iri();
        let mut g = Vec::new();
        let mut o = Out { g: &mut g };
        o.add(
            &e,
            vt::CHART_VERSION,
            NamedNode::new_unchecked(&self.version),
        );
        o.add(&e, vt::MESSAGE_ID, str_lit(&self.message_id));
        o.add(&e, vt::TIMESTAMP, int(self.timestamp_ms));
        o.add(&e, vt::CONFIGURATION_DIGEST, str_lit(&self.digest));
        match &self.body {
            EntryBody::Input { input, effects } => {
                o.add(&e, vt::RDF_TYPE, node(vt::INPUT_ENTRY));
                o.add(&e, vt::RDF_TYPE, node(vt::MACROSTEP));
                let i = address::input_iri(&e);
                o.add(&e, vt::INPUT, i.clone());
                o.add(&i, vt::RDF_TYPE, node(vt::INPUT_EVENT));
                o.add(&i, vt::MESSAGE_ID, str_lit(&input.message_id));
                o.event(&i, &input.event);
                if let Some(c) = &input.correlation {
                    o.add(&i, vt::CORRELATION_ID, str_lit(c));
                }
                if let Some(t) = &input.timer {
                    o.add(&e, vt::FIRED_TIMER, NamedNode::new_unchecked(t));
                }
                for (k, s) in effects.sends.iter().enumerate() {
                    let sn = NamedNode::new_unchecked(&s.iri);
                    o.add(&e, vt::SEND, sn.clone());
                    o.add(&sn, vt::RDF_TYPE, node(vt::OUTGOING_SEND));
                    o.add(&sn, vt::ORDER, uint(k as u64));
                    o.add(&sn, vt::MESSAGE_ID, str_lit(&s.iri));
                    o.add(&sn, vt::SEND_TYPE, str_lit(&s.type_));
                    if let Some(t) = &s.target {
                        o.add(&sn, vt::SEND_TARGET, str_lit(t));
                    }
                    if let Some(r) = &s.recipient {
                        o.add(&sn, vt::RECIPIENT, NamedNode::new_unchecked(r));
                    }
                    if let Some(c) = &s.correlation {
                        o.add(&sn, vt::CORRELATION_ID, str_lit(c));
                    }
                    o.add(&sn, vt::EVENT_NAME, str_lit(&s.event.name));
                    o.add(&sn, vt::PAYLOAD, str_lit(&json::to_text(&s.event.data)));
                    o.add(
                        &sn,
                        vt::ENVELOPE,
                        str_lit(&serde_json::to_string(s).unwrap_or_default()),
                    );
                }
                for t in &effects.timers {
                    o.add(&e, vt::SCHEDULES, NamedNode::new_unchecked(&t.iri));
                    o.timer(t);
                    o.add(
                        &NamedNode::new_unchecked(&t.iri),
                        vt::SCHEDULED_BY,
                        e.clone(),
                    );
                }
                for c in &effects.cancels {
                    o.add(&e, vt::CANCELS, NamedNode::new_unchecked(c));
                }
                if !effects.invokes.is_empty() {
                    let arr = format!("[{}]", effects.invokes.join(","));
                    o.add(&e, vt::INVOKES, str_lit(&arr));
                }
                for r in &effects.requests {
                    o.add(&e, vt::REQUESTS, str_lit(r));
                }
                if let Some(a) = &effects.answers {
                    o.add(&e, vt::ANSWERS, str_lit(a));
                }
                if effects.terminated {
                    o.add(&e, vt::TERMINATED_HERE, boolean(true));
                }
                if let Some(p) = effects.unhandled {
                    o.add(&e, vt::UNHANDLED, str_lit(p.as_str()));
                }
                for t in &effects.transitions {
                    o.add(&e, vt::TOOK_TRANSITION, NamedNode::new_unchecked(t));
                }
            }
            EntryBody::Migration {
                migration,
                from,
                before,
                after,
            } => {
                o.add(&e, vt::RDF_TYPE, node(vt::MIGRATION_ENTRY));
                o.add(&e, vt::MIGRATION_P, NamedNode::new_unchecked(migration));
                o.add(
                    &NamedNode::new_unchecked(migration),
                    vt::RDF_TYPE,
                    node(vt::MIGRATION),
                );
                o.add(&e, vt::FROM_VERSION, NamedNode::new_unchecked(from));
                o.add(&e, vt::TO_VERSION, NamedNode::new_unchecked(&self.version));
                o.add(
                    &e,
                    vt::SNAPSHOT_BEFORE,
                    NamedNode::new_unchecked(&before.iri),
                );
                o.add(&e, vt::SNAPSHOT_AFTER, NamedNode::new_unchecked(&after.iri));
                o.snapshot(before);
                o.snapshot(after);
            }
            EntryBody::Fork {
                source,
                position,
                snapshot,
            } => {
                o.add(&e, vt::RDF_TYPE, node(vt::FORK_ENTRY));
                o.add(&e, vt::FORK_OF, NamedNode::new_unchecked(source));
                o.add(&e, vt::FORK_POSITION, uint(*position));
                o.add(&e, vt::SNAPSHOT_P, NamedNode::new_unchecked(&snapshot.iri));
                o.snapshot(snapshot);
            }
        }
        Entry::new(e, g)
    }

    /// The graph in which the annotations of this entry's transitions are written.
    pub fn annotation_graph(&self) -> NamedNode {
        NamedNode::new_unchecked(format!("{}/annotations", self.iri().as_str()))
    }

    /// The statements `<actor> vt:tookTransition <transition>`, each annotated through a
    /// blank-node reifier (tiramemsu refuses reifier IRIs) with `vt:causedBy <entry>`. They are written to the actor's graph store, in
    /// [`EntryData::annotation_graph`], as RDF 1.2 reifiers when the store supports them and as
    /// `rdf:Statement` reification otherwise (journal entries themselves hold no triple terms).
    // @tg: implements:: [[openspec:runtime-ontology#Triple Term Annotations]]
    pub fn annotations(&self) -> Vec<Annotation> {
        let e = self.iri();
        let g = self.annotation_graph();
        self.effects()
            .map(|eff| {
                eff.transitions
                    .iter()
                    .map(|t| {
                        Annotation::new(
                            g.clone(),
                            Triple::new(
                                self.actor.node().clone(),
                                node(vt::TOOK_TRANSITION),
                                NamedNode::new_unchecked(t),
                            ),
                        )
                        .with(node(vt::CAUSED_BY), e.clone())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Decodes a journal record.
    ///
    /// # Errors
    ///
    /// [`DecodeError`] when a required statement is missing or malformed.
    pub fn from_record(actor: &Address, rec: &JournalRecord) -> Result<Self, DecodeError> {
        let g = rec.entry.triples();
        let e = rec.entry.name().clone();
        let r = Reader { g };
        let version = r.iri(&e, vt::CHART_VERSION)?;
        let message_id = r.string(&e, vt::MESSAGE_ID)?;
        let timestamp_ms = r.int(&e, vt::TIMESTAMP)?;
        let digest = r.string(&e, vt::CONFIGURATION_DIGEST)?;
        let body = if r.has_type(&e, vt::MIGRATION_ENTRY) {
            let before = r.iri(&e, vt::SNAPSHOT_BEFORE)?;
            let after = r.iri(&e, vt::SNAPSHOT_AFTER)?;
            EntryBody::Migration {
                migration: r.iri(&e, vt::MIGRATION_P)?,
                from: r.iri(&e, vt::FROM_VERSION)?,
                before: r.snapshot(&NamedNode::new_unchecked(before))?,
                after: r.snapshot(&NamedNode::new_unchecked(after))?,
            }
        } else if r.has_type(&e, vt::FORK_ENTRY) {
            let s = r.iri(&e, vt::SNAPSHOT_P)?;
            EntryBody::Fork {
                source: r.iri(&e, vt::FORK_OF)?,
                position: u64::try_from(r.int(&e, vt::FORK_POSITION)?)
                    .map_err(|_| DecodeError("negative fork position".into()))?,
                snapshot: r.snapshot(&NamedNode::new_unchecked(s))?,
            }
        } else {
            let i = NamedNode::new_unchecked(r.iri(&e, vt::INPUT)?);
            let input = InputRecord {
                event: r.event(&i)?,
                message_id: r.string(&i, vt::MESSAGE_ID)?,
                correlation: r.opt_string(&i, vt::CORRELATION_ID),
                timer: r.opt_iri(&e, vt::FIRED_TIMER),
            };
            let mut sends = Vec::new();
            for s in r.iris(&e, vt::SEND) {
                let sn = NamedNode::new_unchecked(&s);
                let order = r.int(&sn, vt::ORDER)?;
                let j = r.string(&sn, vt::ENVELOPE)?;
                let rec = serde_json::from_str::<SendRecord>(&j)
                    .map_err(|x| DecodeError(format!("send {s}: {x}")))?;
                sends.push((order, rec));
            }
            sends.sort_by_key(|(o, _)| *o);
            let mut timers = Vec::new();
            for t in r.iris(&e, vt::SCHEDULES) {
                let tn = NamedNode::new_unchecked(&t);
                let j = r.string(&tn, vt::ENVELOPE)?;
                timers.push(
                    serde_json::from_str::<TimerRecord>(&j)
                        .map_err(|x| DecodeError(format!("timer {t}: {x}")))?,
                );
            }
            timers.sort_by(|a, b| a.iri.cmp(&b.iri));
            let mut cancels = r.iris(&e, vt::CANCELS);
            cancels.sort();
            let invokes = match r.opt_string(&e, vt::INVOKES) {
                Some(arr) => serde_json::from_str::<Vec<serde_json::Value>>(&arr)
                    .map_err(|x| DecodeError(format!("invokes: {x}")))?
                    .into_iter()
                    .map(|v| v.to_string())
                    .collect(),
                None => Vec::new(),
            };
            let mut requests = r.strings(&e, vt::REQUESTS);
            requests.sort();
            let mut transitions = r.objects_iri(&e, vt::TOOK_TRANSITION);
            transitions.sort();
            EntryBody::Input {
                input,
                effects: Effects {
                    sends: sends.into_iter().map(|(_, s)| s).collect(),
                    timers,
                    cancels,
                    invokes,
                    requests,
                    answers: r.opt_string(&e, vt::ANSWERS),
                    transitions,
                    terminated: r.opt_bool(&e, vt::TERMINATED_HERE),
                    unhandled: r
                        .opt_string(&e, vt::UNHANDLED)
                        .and_then(|p| UnhandledPolicy::parse(&p)),
                },
            }
        };
        Ok(EntryData {
            actor: actor.clone(),
            position: rec.position.get(),
            version,
            message_id,
            timestamp_ms,
            digest,
            body,
        })
    }
}

/// Reads statements of one graph.
pub(crate) struct Reader<'a> {
    pub(crate) g: &'a Graph,
}

impl Reader<'_> {
    fn objects<'b>(
        &'b self,
        s: &'b NamedNode,
        p: &'static str,
    ) -> impl Iterator<Item = TermRef<'b>> + 'b {
        let p = NamedNodeRef::new_unchecked(p);
        self.g.objects_for_subject_predicate(s, p)
    }

    pub(crate) fn has_type(&self, s: &NamedNode, class: &str) -> bool {
        self.objects(s, vt::RDF_TYPE)
            .any(|o| matches!(o, TermRef::NamedNode(n) if n.as_str() == class))
    }

    pub(crate) fn opt_string(&self, s: &NamedNode, p: &'static str) -> Option<String> {
        self.objects(s, p).find_map(|o| match o {
            TermRef::Literal(l) => Some(l.value().to_owned()),
            _ => None,
        })
    }

    pub(crate) fn string(&self, s: &NamedNode, p: &'static str) -> Result<String, DecodeError> {
        self.opt_string(s, p)
            .ok_or_else(|| DecodeError(format!("{s} has no {p}")))
    }

    fn strings(&self, s: &NamedNode, p: &'static str) -> Vec<String> {
        self.objects(s, p)
            .filter_map(|o| match o {
                TermRef::Literal(l) => Some(l.value().to_owned()),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn int(&self, s: &NamedNode, p: &'static str) -> Result<i64, DecodeError> {
        self.string(s, p)?
            .parse()
            .map_err(|_| DecodeError(format!("{s} {p} is not an integer")))
    }

    fn opt_bool(&self, s: &NamedNode, p: &'static str) -> bool {
        self.opt_string(s, p)
            .is_some_and(|v| v == "true" || v == "1")
    }

    pub(crate) fn opt_iri(&self, s: &NamedNode, p: &'static str) -> Option<String> {
        self.objects(s, p).find_map(|o| match o {
            TermRef::NamedNode(n) => Some(n.as_str().to_owned()),
            _ => None,
        })
    }

    pub(crate) fn iri(&self, s: &NamedNode, p: &'static str) -> Result<String, DecodeError> {
        self.opt_iri(s, p)
            .ok_or_else(|| DecodeError(format!("{s} has no {p}")))
    }

    fn iris(&self, s: &NamedNode, p: &'static str) -> Vec<String> {
        self.objects(s, p)
            .filter_map(|o| match o {
                TermRef::NamedNode(n) => Some(n.as_str().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn objects_iri(&self, s: &NamedNode, p: &'static str) -> Vec<String> {
        let set: BTreeSet<String> = self.iris(s, p).into_iter().collect();
        set.into_iter().collect()
    }

    fn event(&self, s: &NamedNode) -> Result<Event, DecodeError> {
        let data = json::from_text(&self.string(s, vt::PAYLOAD)?)
            .map_err(|e| DecodeError(format!("{s} payload: {e}")))?;
        Ok(Event {
            name: self.string(s, vt::EVENT_NAME)?,
            type_: event_type_of(&self.opt_string(s, vt::EVENT_TYPE).unwrap_or_default()),
            sendid: self.opt_string(s, vt::SEND_ID),
            origin: self.opt_string(s, vt::ORIGIN),
            origintype: self.opt_string(s, vt::ORIGIN_TYPE),
            invokeid: self.opt_string(s, vt::INVOKE_ID),
            data,
        })
    }

    pub(crate) fn snapshot(&self, n: &NamedNode) -> Result<SnapshotData, DecodeError> {
        Ok(SnapshotData {
            iri: n.as_str().to_owned(),
            actor: self.iri(n, vt::SNAPSHOT_OF)?,
            position: u64::try_from(self.int(n, vt::POSITION)?)
                .map_err(|_| DecodeError("negative snapshot position".into()))?,
            version: self.iri(n, vt::CHART_VERSION)?,
            digest: self.string(n, vt::CONFIGURATION_DIGEST)?,
            session: self.string(n, vt::SESSION_STATE)?,
            datamodel: self.string(n, vt::DATAMODEL_STATE)?,
            runtime: self.string(n, vt::RUNTIME_STATE)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vertep_store::Position;

    fn sample() -> EntryData {
        let a = Address::parse("urn:vertep:test:actor").unwrap();
        let e = a.entry_iri(1);
        EntryData {
            actor: a.clone(),
            position: 1,
            version: "urn:vertep:chart:sha256:00".into(),
            message_id: "m1".into(),
            timestamp_ms: 1_000,
            digest: "d".into(),
            body: EntryBody::Input {
                input: InputRecord {
                    event: Event {
                        name: "go".into(),
                        data: Value::Object(vec![("x".into(), Value::Number(1.0))]),
                        origin: Some("urn:vertep:test:sender".into()),
                        ..Event::default()
                    },
                    message_id: "m1".into(),
                    correlation: None,
                    timer: None,
                },
                effects: Effects {
                    sends: vec![SendRecord {
                        iri: address::send_iri(&e, 0).as_str().into(),
                        send_id: "s1".into(),
                        event: Event::named("out", EventType::External),
                        target: Some("urn:vertep:test:b".into()),
                        recipient: Some("urn:vertep:test:b".into()),
                        type_: "scxml".into(),
                        correlation: None,
                    }],
                    timers: vec![TimerRecord {
                        iri: address::timer_iri(&e, "r1").as_str().into(),
                        actor: a.as_str().into(),
                        send_id: "r1".into(),
                        due_ms: 6_000,
                        event: Event::named("remind", EventType::External),
                        recipient: a.as_str().into(),
                        reminder: None,
                        correlation: None,
                    }],
                    transitions: vec!["urn:vertep:chart:sha256:00#s.a:t0".into()],
                    ..Effects::default()
                },
            },
        }
    }

    #[test]
    fn entry_round_trips_through_rdf() {
        let d = sample();
        let entry = d
            .to_entry()
            .stamp(d.actor.node(), Position::new(1))
            .unwrap();
        let rec = JournalRecord {
            position: Position::new(1),
            entry,
        };
        assert_eq!(EntryData::from_record(&d.actor, &rec).unwrap(), d);
        assert_eq!(d.annotations().len(), 1);
    }

    #[test]
    fn digest_depends_on_configuration_and_data() {
        let a = digest(&["s1".into()], &Value::Undefined, false);
        assert_ne!(a, digest(&["s2".into()], &Value::Undefined, false));
        assert_ne!(a, digest(&["s1".into()], &Value::Null, false));
        assert_eq!(a, digest(&["s1".into()], &Value::Undefined, false));
    }
}
