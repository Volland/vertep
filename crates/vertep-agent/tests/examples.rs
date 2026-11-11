//! The support-agent examples run end to end: the ReAct loop, its iteration bound, and a fork
//! from a journal position with an injected decision.

#[path = "../examples/support/kit.rs"]
mod kit;

use vertep_agent::decider::Annotations;
use vertep_agent::journal::AgentInput;
use vertep_agent::llm::ScriptedProvider;
use vertep_core::datamodel::Value;

fn json(v: serde_json::Value) -> Value {
    vertep_agent::json::from_json(&v)
}

fn count(host: &vertep_agent::AgentHost, actor: &str, kind: &str) -> usize {
    host.journal(actor)
        .iter()
        .filter(|e| e.input.kind() == kind)
        .count()
}

// @tg: verifies:: [[openspec:llm-actors#LLM as Main Driver#ReAct loop runs to completion]] {level: e2e}
#[test]
fn react_loop_runs_to_completion() {
    let model = kit::script();
    let (mut host, _) = kit::host(model.clone(), kit::triage());
    let v = host.register(kit::chart()).unwrap();
    host.spawn(kit::AGENT, &v).unwrap();
    host.send(kit::AGENT, kit::task());
    host.run_until_idle();
    let s = host.state(kit::AGENT).unwrap();
    assert!(s.terminated);
    assert_eq!(
        s.done,
        Some(json(serde_json::json!({
            "answer": { "text": "Your refund for A-17 is on its way.", "confidence": 0.95 },
            "steps": 2
        })))
    );
    assert_eq!(
        count(&host, kit::AGENT, "tool.result"),
        2,
        "two tool requests"
    );
    assert_eq!(
        count(&host, kit::AGENT, "decision"),
        3,
        "three journaled decisions"
    );
    assert_eq!(model.calls(), 3);
    assert_eq!(host.stats().tool_calls.len(), 2);
    // every model call is accounted: the triage call and the three decisions
    let usage = host.usage(kit::AGENT).unwrap();
    assert_eq!(usage.calls, 4);
    assert_eq!(usage.prompt_tokens, 120 + 3 * 800);
    // the provenance graph conforms to the agent shapes
    let g = host.graph(kit::AGENT).unwrap();
    assert!(vertep_agent::ontology::validate(&g).conforms());
}

// @tg: verifies:: [[openspec:llm-actors#LLM as Main Driver#Iteration bound enforced]] {level: e2e}
#[test]
fn iteration_bound_enforced() {
    // a model that never finishes
    let model = ScriptedProvider::new().with_default(kit::answer(&format!(
        r#"{{"event":"call_tool","payload":{{"tool":"{}","args":{{"query":"again"}}}}}}"#,
        kit::SEARCH
    )));
    let (mut host, _) = kit::host(model.clone(), kit::triage());
    let v = host.register(kit::chart()).unwrap();
    host.spawn(kit::AGENT, &v).unwrap();
    host.send(kit::AGENT, kit::task());
    host.run_until_idle();
    let s = host.state(kit::AGENT).unwrap();
    assert_eq!(
        s.done,
        Some(json(serde_json::json!({ "steps": 10 }))),
        "budget_out was reached"
    );
    assert_eq!(count(&host, kit::AGENT, "decision"), 10);
    assert_eq!(
        model.calls(),
        10,
        "the eleventh entry does not consult the model"
    );
}

// @tg: verifies:: [[openspec:llm-actors#Journaled Model Results#Forked run reuses history]] {level: e2e}
#[test]
fn forked_run_reuses_history() {
    let model = kit::script();
    let triage = kit::triage();
    let (mut host, _) = kit::host(model.clone(), triage.clone());
    let v = host.register(kit::chart()).unwrap();
    host.spawn(kit::AGENT, &v).unwrap();
    host.send(kit::AGENT, kit::task());
    host.run_until_idle();
    let tools_before: usize = host.stats().tool_calls.values().sum();
    let (triage_before, model_before) = (triage.calls(), model.calls());

    // fork right after the first tool result, where the agent was deciding again
    let n = host
        .journal(kit::AGENT)
        .iter()
        .find(|e| matches!(e.input, AgentInput::ToolResult { .. }))
        .unwrap()
        .position;
    let fork = format!("{}/fork", kit::AGENT);
    host.fork(kit::AGENT, n, &fork, Vec::new()).unwrap();
    let offer = host
        .offers(&fork)
        .pop()
        .expect("the offer open at N is reproduced");
    assert!(offer.correlation.starts_with(&fork));
    host.deliver(
        &fork,
        AgentInput::Decision {
            correlation: offer.correlation,
            event: "finish".into(),
            payload: json(serde_json::json!({ "text": "Escalated." })),
            annotations: Annotations::default(),
        },
    );
    host.run_until_idle();

    let s = host.state(&fork).unwrap();
    assert_eq!(
        s.done,
        Some(json(
            serde_json::json!({ "answer": { "text": "Escalated." }, "steps": 1 })
        ))
    );
    // entries 1..N are the original's, so its model result came from the journal
    let original = host.journal(kit::AGENT);
    let forked = host.journal(&fork);
    assert_eq!(
        forked[..n as usize]
            .iter()
            .map(|e| e.input.kind())
            .collect::<Vec<_>>(),
        original[..n as usize]
            .iter()
            .map(|e| e.input.kind())
            .collect::<Vec<_>>()
    );
    assert_eq!(count(&host, &fork, "llm.result"), 1);
    assert_eq!(
        triage.calls(),
        triage_before,
        "the triage call was not repeated"
    );
    let tools_after: usize = host.stats().tool_calls.values().sum();
    assert_eq!(tools_after, tools_before, "no tool ran again");
    // only the re-offer after the fork reached a model
    assert_eq!(model.calls(), model_before + 1);
    // the original run is untouched
    assert_eq!(
        host.state(kit::AGENT).unwrap().done.unwrap(),
        json(serde_json::json!({
            "answer": { "text": "Your refund for A-17 is on its way.", "confidence": 0.95 },
            "steps": 2
        }))
    );
}
