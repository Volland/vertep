//! Forks a support-agent run from a journal position and injects a different decision.
//!
//! The original run searches the knowledge base, looks up the order and finishes. The fork
//! starts from the position right after the search result and finishes immediately instead.
//! History up to the fork is replayed from the journal: the triage model and the tools are not
//! called again. Run with `cargo run -p vertep-agent --example fork_from_journal`.

#[path = "support/kit.rs"]
mod kit;

use vertep_agent::decider::Annotations;
use vertep_agent::journal::AgentInput;

fn main() {
    let triage = kit::triage();
    let (mut host, _clock) = kit::host(kit::script(), triage.clone());
    let version = host.register(kit::chart()).expect("an agent chart");
    host.spawn(kit::AGENT, &version).expect("the agent starts");
    host.send(kit::AGENT, kit::task());
    host.run_until_idle();
    println!("original: {:?}", host.state(kit::AGENT).unwrap().done);

    // the position of the first tool result
    let at = host
        .journal(kit::AGENT)
        .iter()
        .find(|e| matches!(e.input, AgentInput::ToolResult { .. }))
        .map(|e| e.position)
        .expect("the run called a tool");
    let fork = format!("{}/fork-1", kit::AGENT);
    println!(
        "forking at {at} in {:?}",
        host.state_at(kit::AGENT, at).unwrap().configuration
    );
    let calls = triage.calls();
    host.fork(kit::AGENT, at, &fork, Vec::new())
        .expect("the fork starts");
    // the forked actor reproduces the offer that was open at that position, under its own IRI
    let offer = host.offers(&fork).pop().expect("an open offer");
    println!(
        "open offer {} with {:?}",
        offer.correlation,
        offer
            .candidates
            .iter()
            .map(|c| &c.event)
            .collect::<Vec<_>>()
    );
    host.deliver(
        &fork,
        AgentInput::Decision {
            correlation: offer.correlation,
            event: "finish".into(),
            payload: vertep_agent::json::from_json(
                &serde_json::json!({ "text": "Escalated by a supervisor." }),
            ),
            annotations: Annotations {
                rationale: Some("injected by the operator".into()),
                ..Annotations::default()
            },
        },
    );
    host.run_until_idle();
    println!("fork: {:?}", host.state(&fork).unwrap().done);
    println!("triage calls after the fork: {}", triage.calls() - calls);
}
