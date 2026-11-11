//! The pieces of the support-agent examples: the chart, two tools, the mock model and a host
//! on a virtual clock. Shared by the examples and by `tests/examples.rs`.
#![allow(dead_code)]

use std::sync::Arc;

use vertep_agent::host::AgentHost;
use vertep_agent::llm::{ProviderResult, ScriptedProvider, Usage};
use vertep_agent::tool::Tool;
use vertep_agent::{GuardDecider, LlmDecider};
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;
use vertep_runtime::VirtualClock;

/// The knowledge-base search tool.
pub const SEARCH: &str = "https://example.org/tools/kb-search";
/// The order lookup tool.
pub const ORDERS: &str = "https://example.org/tools/order-lookup";
/// The agent actor.
pub const AGENT: &str = "https://example.org/agents/support/42";
/// 2026-11-11T00:00:00Z.
pub const T0: i64 = 1_794_355_200_000;

/// The ReAct support-agent chart.
pub fn chart() -> Chart {
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(include_str!("../support-agent.scxml"), &opts)
        .expect("the support-agent chart reads")
        .chart
}

fn json(v: serde_json::Value) -> Value {
    vertep_agent::json::from_json(&v)
}

/// The two tools, typed in the IR type language.
pub fn tools() -> Vec<Tool> {
    let search = Tool::new(
        SEARCH,
        "type Article = { title: string, body: string };",
        "{ query: string }",
        "list<Article>",
        |args: Value| async move {
            let q = match vertep_agent::json::field(&args, "query") {
                Some(Value::String(q)) => q.clone(),
                _ => return Err("no query".to_owned()),
            };
            Ok(json(serde_json::json!([{
                "title": "Refunds",
                "body": format!("Refunds for '{q}' are issued within 5 days of the return."),
            }])))
        },
    )
    .expect("the search tool is well-typed")
    .with_description("Search the support knowledge base");
    let orders = Tool::new(
        ORDERS,
        "type Order = { id: string, status: string };",
        "{ id: string }",
        "Order",
        |args: Value| async move {
            let id = match vertep_agent::json::field(&args, "id") {
                Some(Value::String(id)) => id.clone(),
                _ => return Err("no id".to_owned()),
            };
            Ok(json(serde_json::json!({ "id": id, "status": "returned" })))
        },
    )
    .expect("the order tool is well-typed")
    .with_description("Look up an order by id");
    vec![search, orders]
}

/// A model answer as the mock provider returns it.
pub fn answer(content: &str) -> Result<ProviderResult, vertep_agent::ProviderError> {
    Ok(ProviderResult::text(
        content,
        "claude-opus-5-5",
        Usage {
            prompt_tokens: 800,
            completion_tokens: 60,
        },
    ))
}

/// The model's decisions for the happy path: search, look up the order, finish.
pub fn script() -> ScriptedProvider {
    let model = ScriptedProvider::new();
    model.push(answer(&format!(
        r#"{{"event":"call_tool","payload":{{"tool":"{SEARCH}","args":{{"query":"refund"}}}},"rationale":"check the policy first","confidence":0.9}}"#
    )));
    model.push(answer(&format!(
        r#"{{"event":"call_tool","payload":{{"tool":"{ORDERS}","args":{{"id":"A-17"}}}},"rationale":"is the item back?","confidence":0.8}}"#
    )));
    model.push(answer(
        r#"{"event":"finish","payload":{"text":"Your refund for A-17 is on its way.","confidence":0.95},"rationale":"policy and order agree","confidence":0.95}"#,
    ));
    model
}

/// A host with the triage provider, the LLM decider over `model`, a guard fallback, the two
/// tools, on a virtual clock.
pub fn host(model: ScriptedProvider, triage: ScriptedProvider) -> (AgentHost, VirtualClock) {
    let clock = VirtualClock::new(T0);
    let mut b = AgentHost::builder(Arc::new(clock.clone()))
        .provider("triage", Arc::new(triage))
        .decider(
            "llm/claude",
            Arc::new(
                LlmDecider::new("claude", Arc::new(model), "claude-opus-5-5").with_instructions(
                    "You are a support agent. Use the tools, then finish with an answer.",
                ),
            ),
        )
        .decider(
            "human/support",
            Arc::new(GuardDecider::with_rules(vec![
                vertep_agent::decider::GuardRule {
                    event: "finish".into(),
                    guard: None,
                    payload: Some("{'text': 'A person will get back to you.'}".into()),
                },
            ])),
        );
    for t in tools() {
        b = b.tool(t);
    }
    (b.build(), clock)
}

/// The triage model: one line per ticket.
pub fn triage() -> ScriptedProvider {
    ScriptedProvider::new().with_default(Ok(ProviderResult::text(
        "refund request for a returned order",
        "claude-opus-5-5",
        Usage {
            prompt_tokens: 120,
            completion_tokens: 9,
        },
    )))
}

/// The task event.
pub fn task() -> vertep_core::datamodel::Event {
    vertep_core::datamodel::Event {
        name: "task".into(),
        data: json(
            serde_json::json!({ "id": "T-1", "text": "Where is my refund for order A-17?" }),
        ),
        ..Default::default()
    }
}
