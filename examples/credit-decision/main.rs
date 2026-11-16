//! Credit decision: a regulated decision that can be explained from its record.
//!
//! A lender uses a model to underwrite consumer loans. The chart (`loan.scxml`) holds the
//! policy: a hard debt-to-income limit, an approval ceiling the model cannot talk its way past,
//! and a human officer for the cases it refers. A scripted decider stands in for the model so
//! the example needs no network and prints the same thing on every run.
//!
//! Three applications are decided, then a second host opens the same journal with **no decider
//! bound** and answers the question a regulator or an applicant asks afterwards: what was the
//! process in, what was offered, who chose, and why? Nothing is reconstructed from logs: the
//! journal is the process.
//!
//! Illustrates the `deciders` capability (Enabled Event Offer, Decisions Are Journaled Inputs,
//! Decider Safety).
//!
//! Run with `cargo run -p vertep-examples --bin credit-decision`.

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
const LOAN: &str = include_str!("loan.scxml");
/// 2026-11-11T00:00:00Z, the virtual clock's start.
const T0: i64 = 1_794_355_200_000;

type Error = Box<dyn std::error::Error>;

/// The applications: id, amount, debt-to-income percent.
const APPLICATIONS: [(&str, u32, u32); 3] = [
    ("https://example.org/loans/1001", 20_000, 31),
    ("https://example.org/loans/1002", 80_000, 38),
    ("https://example.org/loans/1003", 15_000, 62),
];

/// A deterministic stand-in for a model or a person: answers from a script, in order, and says
/// what it was offered.
struct Scripted {
    role: &'static str,
    answers: Mutex<VecDeque<(&'static str, &'static str)>>,
}

impl Decider for Scripted {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Custom
    }

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
                    "  {} asked in `{}`: offered {offered:?} -> {event}",
                    self.role, request.state
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

    // ---- decide three applications ----------------------------------------------------------
    println!("deciding three applications:");
    let underwriter = Scripted {
        role: "underwriter",
        answers: Mutex::new(VecDeque::from([
            ("approve", "stable income, low ratio, small amount"),
            ("approve", "strong credit history"),
        ])),
    };
    let officer = Scripted {
        role: "officer    ",
        answers: Mutex::new(VecDeque::from([(
            "approve",
            "verified employment by phone; collateral accepted",
        )])),
    };
    let mut host = AgentHost::builder(Arc::new(VirtualClock::new(T0)))
        .journal(journal.clone())
        .decider("underwriter", Arc::new(underwriter))
        .decider("officer", Arc::new(officer))
        .build();
    let version = host.register(chart()?)?;
    for (id, amount, dti) in APPLICATIONS {
        println!("{id}  amount {amount}, ratio {dti}%");
        host.spawn(id, &version)?;
        host.send(id, apply(amount, dti));
        host.run_until_idle();
        println!("  -> {:?}", host.state(id)?.configuration);
    }

    // ---- explain, from the journal alone ----------------------------------------------------
    println!();
    println!("an auditor opens the same journal; no decider is bound:");
    let mut audit = AgentHost::builder(Arc::new(VirtualClock::new(T0)))
        .journal(journal)
        .build();
    audit.register(chart()?)?;
    for (id, _, _) in APPLICATIONS {
        let state = audit.state(id)?;
        println!("{id}  final {:?}", state.configuration);
        for entry in audit.journal(id) {
            let detail = match &entry.input {
                AgentInput::Start { .. } => String::new(),
                AgentInput::Event { event, .. } => event.name.clone(),
                AgentInput::Decision {
                    event, annotations, ..
                } => format!(
                    "{event} [{}, offered {:?}], rationale {:?}",
                    entry.disposition_or_unknown(),
                    entry.offered.clone().unwrap_or_default(),
                    annotations.rationale.as_deref().unwrap_or("")
                ),
                other => format!("{other:?}"),
            };
            let line = format!("  {} {:<9} {detail}", entry.position, entry.input.kind());
            println!("{}", line.trim_end());
        }
        for position in 2..=state.position {
            println!(
                "  as of entry {position}: {:?}",
                audit.state_at(id, position)?.configuration
            );
        }
    }
    // ---- export what an auditor can verify on another machine ---------------------------------
    let referred = APPLICATIONS[1].0;
    let last = audit.journal(referred).len() as u64;
    let bundle = audit.export_bundle(referred, 1, last)?;
    let text = bundle.to_json();
    let verified = vertep::agent::bundle::Bundle::from_json(&text)?.verify()?;
    println!(
        "bundle of {referred}: {} entries, {} bytes; verified offline: {} explanations recomputed, {} decider calls",
        bundle.entries.len(),
        text.len(),
        verified.explanations.len(),
        verified.decider_calls
    );
    println!(
        "entries replayed: {}, decider calls: {}",
        audit.stats().replayed,
        audit.stats().decider_calls.values().sum::<usize>()
    );
    Ok(())
}

/// Reads the chart. Charts naming a datamodel other than `null` need `AnyDatamodel`.
fn chart() -> Result<vertep::prelude::Chart, Error> {
    let opts = vertep::scxml::ReadOptions::new().with_datamodels(&vertep::scxml::AnyDatamodel);
    Ok(vertep::scxml::read(LOAN, &opts)?.chart)
}

/// An `apply` event with the application's figures.
fn apply(amount: u32, dti: u32) -> Event {
    Event {
        name: "apply".into(),
        data: vertep::agent::json::from_json(&serde_json::json!({ "amount": amount, "dti": dti })),
        ..Event::default()
    }
}
