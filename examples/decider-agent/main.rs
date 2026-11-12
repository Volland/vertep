//! Decider agent: choice points decided by a pluggable decider, journaled, and replayed
//! without it.
//!
//! The refund chart has two decide states. A real deployment would bind `policy` to an
//! `LlmDecider` (or a person's inbox); here a scripted decider answers from a fixed list, so
//! the example needs no network and no API key, and prints the same thing on every run.
//!
//! Run 1 sends a refund request and lets the decider choose twice. Every answer becomes a
//! journal entry (`decision`, with its correlation id and rationale) before the chart processes
//! it. Run 2 opens a second host on the same journal with **no decider bound at all**: replaying
//! the journal reproduces the same states and context, and the decider is never asked.
//!
//! Illustrates the `deciders` capability (Decider Contract, Enabled Event Offer, Decisions Are
//! Journaled Inputs).
//!
//! Run with `cargo run -p vertep-examples --bin decider-agent`.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use vertep::agent::{
    AgentHost, AgentInput, Annotations, Decider, DeciderKind, Decision, DecisionOutcome,
    DecisionRequest, MemoryJournal,
};
use vertep::prelude::{Event, VirtualClock};

/// The chart, next to this file.
const REFUND: &str = include_str!("refund.scxml");
/// The agent actor's IRI.
const AGENT: &str = "https://example.org/agents/refund/7";
/// 2026-11-11T00:00:00Z, the virtual clock's start.
const T0: i64 = 1_794_355_200_000;

type Error = Box<dyn std::error::Error>;

/// A deterministic stand-in for an LLM: answers from a script, in order, and prints each offer.
struct ScriptedDecider {
    answers: Mutex<VecDeque<(&'static str, &'static str)>>,
}

impl Decider for ScriptedDecider {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Custom
    }

    /// The request carries the decide state, the candidate events (only those enabled now),
    /// a read-only view of the context and the correlation id the answer must carry.
    fn decide(
        &self,
        request: DecisionRequest,
    ) -> Pin<Box<dyn Future<Output = DecisionOutcome> + Send>> {
        let offered: Vec<&str> = request
            .candidates
            .iter()
            .map(|c| c.event.as_str())
            .collect();
        let answer = self.answers.lock().expect("lock").pop_front();
        let outcome = match answer {
            Some((event, why)) => {
                println!(
                    "  decider asked in `{}`: offered {offered:?} -> {event}",
                    request.state
                );
                let mut decision = Decision::new(event, vertep::prelude::Value::Undefined);
                decision.annotations = Annotations {
                    rationale: Some(why.to_owned()),
                    ..Annotations::default()
                };
                DecisionOutcome::Decision(decision)
            }
            None => DecisionOutcome::refuse("the script is exhausted"),
        };
        Box::pin(async move { outcome })
    }
}

fn main() -> Result<(), Error> {
    let journal = Arc::new(MemoryJournal::new());

    // ---- run 1: with the decider ------------------------------------------------------------
    println!("run 1, decider bound to `policy`:");
    let decider = ScriptedDecider {
        answers: Mutex::new(VecDeque::from([
            ("escalate", "amount is above my approval limit"),
            ("approve", "manager approved: item returned unopened"),
        ])),
    };
    let mut host = AgentHost::builder(Arc::new(VirtualClock::new(T0)))
        .journal(journal.clone())
        .decider("policy", Arc::new(decider))
        .build();
    let version = host.register(chart()?)?;
    host.spawn(AGENT, &version)?;
    host.send(AGENT, request(250));
    host.run_until_idle();

    let before = host.state(AGENT)?;
    println!(
        "  final: {:?} context={}",
        before.configuration,
        context(&before)
    );
    println!("journal:");
    for entry in host.journal(AGENT) {
        let detail = match &entry.input {
            AgentInput::Start { .. } => String::new(),
            AgentInput::Event { event, .. } => event.name.clone(),
            AgentInput::Decision {
                correlation,
                event,
                annotations,
                ..
            } => format!(
                "{event} (offer {correlation}, rationale {:?})",
                annotations.rationale.as_deref().unwrap_or("")
            ),
            other => format!("{other:?}"),
        };
        let line = format!("  {} {:<9} {detail}", entry.position, entry.input.kind());
        println!("{}", line.trim_end());
    }

    // ---- run 2: same journal, no decider ----------------------------------------------------
    println!("run 2, same journal, no decider bound:");
    let mut replay = AgentHost::builder(Arc::new(VirtualClock::new(T0)))
        .journal(journal)
        .build();
    replay.register(chart()?)?;
    let after = replay.state(AGENT)?; // activates the actor by replaying the journal
    replay.run_until_idle();
    println!(
        "  final: {:?} context={}",
        after.configuration,
        context(&after)
    );
    for position in 2..=after.position {
        let at = replay.state_at(AGENT, position)?;
        println!("  as of entry {position}: {:?}", at.configuration);
    }
    println!(
        "  entries replayed: {}, decider calls: {}",
        replay.stats().replayed,
        replay.stats().decider_calls.values().sum::<usize>()
    );
    assert_eq!(after.configuration, before.configuration);
    assert_eq!(after.datamodel, before.datamodel);
    Ok(())
}

/// Reads the chart. Charts naming a datamodel other than `null` need `AnyDatamodel`.
fn chart() -> Result<vertep::prelude::Chart, Error> {
    let opts = vertep::scxml::ReadOptions::new().with_datamodels(&vertep::scxml::AnyDatamodel);
    Ok(vertep::scxml::read(REFUND, &opts)?.chart)
}

/// A `request` event with an `amount` payload.
fn request(amount: u32) -> Event {
    Event {
        name: "request".into(),
        data: vertep::agent::json::from_json(&serde_json::json!({ "amount": amount })),
        ..Event::default()
    }
}

/// The context as JSON.
fn context(state: &vertep::agent::AgentState) -> String {
    let view = vertep::agent::ContextView::new(state.datamodel.clone());
    vertep::agent::json::to_json(view.data()).to_string()
}
