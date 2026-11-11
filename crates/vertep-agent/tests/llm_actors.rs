//! The `llm-actors` capability: LLM invocations, prompt templates, providers, structured
//! output, journaled results, retries, streaming, tools, grants, accounting and the agent
//! ontology module.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use vertep_agent::grant::{self, Grant, GrantedActor};
use vertep_agent::host::AgentHost;
use vertep_agent::journal::{AgentInput, MemoryJournal};
use vertep_agent::llm::{ErrorClass, ProviderError, ScriptedProvider, render};
use vertep_agent::machine::{AgentChart, ChartError};
use vertep_agent::tool::Tool;
use vertep_agent::vocab;
use vertep_core::datamodel::Value;
use vertep_core::ir::Chart;

const LLM_HEAD: &str = r#"<datamodel><data id="customer" expr="{'name': 'Ada'}"/><data id="summary" expr="''"/>
  <data id="reason" expr="''"/><data id="class" expr="''"/><data id="retry" expr="0"/>
  <data id="attempts" expr="0"/><data id="buffer" expr="''"/><data id="category" expr="''"/></datamodel>"#;

/// `idle` --go--> `work`, which invokes the LLM call `summarise` declared by `attrs`.
fn llm_chart(attrs: &str) -> Chart {
    cel("idle", LLM_HEAD, &llm_body(attrs))
}

fn llm_body(attrs: &str) -> String {
    format!(
        r#"<state id="idle"><transition event="go" target="work"/></state>
  <state id="work">
    <invoke type="vt:llm" id="summarise" {attrs}/>
    <transition event="done.invoke.summarise" target="done"><assign location="summary" expr="ev.data.text"/></transition>
    <transition event="error.execution" target="failed"><assign location="reason" expr="ev.data.reason"/></transition>
    <transition event="error.llm" target="failed"><assign location="class" expr="ev.data['class']"/><assign location="attempts" expr="ev.data.attempts"/></transition>
    <transition event="vt.llm.partial"><assign location="buffer" expr="ctx.buffer + ev.data.delta"/></transition>
    <transition event="leave" target="idle"/>
  </state>
  <state id="done"/><state id="failed"/>"#
    )
}

const SUMMARISE: &str = r#"vt:provider="p" vt:model="m1" vt:template="Summarise the case of {{context.customer.name}}.""#;

fn llm_host(
    provider: Arc<dyn vertep_agent::llm::Provider>,
) -> (AgentHost, vertep_runtime::VirtualClock) {
    let (b, clock) = builder();
    let mut host = b.provider("p", provider).build();
    spawn(&mut host, llm_chart(SUMMARISE));
    (host, clock)
}

fn llm_inputs(host: &AgentHost) -> Vec<(u64, AgentInput)> {
    host.journal(AGENT)
        .into_iter()
        .filter(|e| {
            matches!(
                e.input,
                AgentInput::LlmResult { .. } | AgentInput::LlmError { .. }
            )
        })
        .map(|e| (e.position, e.input))
        .collect()
}

// ----- LLM Invocation -------------------------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#LLM Invocation#Successful LLM call]] {level: integration}
#[test]
fn successful_llm_call() {
    let p = ScriptedProvider::new();
    p.push_text("Ada wants a refund.", 50, 7);
    let (mut host, _) = llm_host(Arc::new(p.clone()));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["done"]);
    assert_eq!(
        ctx(&mut host, "summary"),
        Some(Value::String("Ada wants a refund.".into()))
    );
    assert_eq!(p.calls(), 1);
    assert_eq!(p.requests()[0].model, "m1");
}

// @tg: verifies:: [[openspec:llm-actors#LLM Invocation#Call cancelled on state exit]] {level: integration}
#[test]
fn call_cancelled_on_state_exit() {
    let gate = Gate::default();
    let (mut host, _) = llm_host(Arc::new(gate.clone()));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(gate.in_flight(), 1);
    host.send(AGENT, ev("leave"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["idle"]);
    // the in-flight call was aborted: its answer has nowhere to go
    assert!(!gate.release(Ok(text("late", "m1", 1, 1))));
    host.run_until_idle();
    // a late result delivered anyway is dropped as belonging to a cancelled invocation
    host.deliver(
        AGENT,
        AgentInput::LlmResult {
            invokeid: "summarise".into(),
            attempt: 1,
            provider: "p".into(),
            prompt_hash: "sha256:00".into(),
            content: "late".into(),
            usage: Default::default(),
            model: "m1".into(),
            latency_ms: 0,
            cost: None,
            tool_calls: Vec::new(),
        },
    );
    host.run_until_idle();
    assert!(llm_inputs(&host).is_empty());
    assert_eq!(config(&mut host), vec!["idle"]);
    assert_eq!(
        ctx(&mut host, "summary"),
        Some(Value::String(String::new()))
    );
}

// @tg: verifies:: [[openspec:llm-actors#LLM Invocation#Unknown provider binding]] {level: integration}
#[test]
fn unknown_provider_binding() {
    let (b, _) = builder();
    let mut host = b.build();
    spawn(&mut host, llm_chart(SUMMARISE));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("unknown-provider".into()))
    );
    assert!(host.stats().provider_calls.is_empty());
}

// ----- Prompt Templates -----------------------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#Prompt Templates#Template rendered from context]] {level: integration}
#[test]
fn template_rendered_from_context() {
    let p = ScriptedProvider::new();
    p.push_text("ok", 1, 1);
    let (mut host, _) = llm_host(Arc::new(p.clone()));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    let prompt = &p.requests()[0].prompt;
    assert_eq!(prompt.text, "Summarise the case of Ada.");
    let (_, AgentInput::LlmResult { prompt_hash, .. }) = &llm_inputs(&host)[0] else {
        panic!("a result")
    };
    use sha2::Digest;
    let hex: String = sha2::Sha256::digest(prompt.text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(prompt_hash, &format!("sha256:{hex}"));
}

// @tg: verifies:: [[openspec:llm-actors#Prompt Templates#Missing template variable]] {level: integration}
#[test]
fn missing_template_variable() {
    let p = ScriptedProvider::new();
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(p.clone())).build();
    spawn(
        &mut host,
        llm_chart(r#"vt:provider="p" vt:template="Hi {{context.customer.surname}}""#),
    );
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "reason"),
        Some(Value::String("template".into()))
    );
    assert_eq!(p.calls(), 0, "no model call is made");
    // the missing path travels with the error
    let e = render(
        "{{context.customer.surname}}",
        &val(serde_json::json!({"customer": {}})),
        None,
    )
    .unwrap_err();
    assert_eq!(e.path, "context.customer.surname");
}

// @tg: verifies:: [[openspec:llm-actors#Prompt Templates#Deterministic rendering]] {level: unit}
#[test]
fn deterministic_rendering() {
    let ctx = val(serde_json::json!({"a": {"z": 1, "b": [true, null]}, "name": "Ada"}));
    let event = ev_data("task", val(serde_json::json!({"y": 2, "x": "q"})));
    let t = "{{context.name}} {{context.a}} {{event.name}} {{event.data}}";
    let one = render(t, &ctx, Some(&event)).unwrap();
    let two = render(t, &ctx.clone(), Some(&event.clone())).unwrap();
    assert_eq!(one.hash, two.hash);
    assert_eq!(
        one.text,
        r#"Ada {"b":[true,null],"z":1} task {"x":"q","y":2}"#
    );
}

// ----- Provider Contract ----------------------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#Provider Contract#Two providers interchangeable]] {level: integration}
#[test]
fn two_providers_interchangeable() {
    let run = |binding: &str, model: &str, usage: (u64, u64)| {
        let p = ScriptedProvider::new();
        p.push(Ok(text(
            r#"{"category":"tech","urgency":2}"#,
            model,
            usage.0,
            usage.1,
        )));
        let (b, _) = builder();
        let mut host = b.provider(binding, Arc::new(p)).build();
        spawn(
            &mut host,
            llm_chart(&format!(
                r#"vt:provider="{binding}" vt:template="classify" vt:output="{{ category: string, urgency: int }}""#
            ))
            .replace_summary(),
        );
        host.send(AGENT, ev("go"));
        host.run_until_idle();
        let s = host.state(AGENT).unwrap();
        let journaled = llm_inputs(&host);
        // the context apart from the read-only accounting variable
        let mut data = vertep_agent::ContextView::new(s.datamodel).data().clone();
        if let Value::Object(f) = &mut data {
            f.retain(|(k, _)| k != vocab::USAGE_VARIABLE);
        }
        (s.configuration, data, journaled, host.usage(AGENT).unwrap())
    };
    let a = run("a", "model-a", (10, 3));
    let b = run("b", "model-b", (99, 30));
    assert_eq!(
        (a.0.clone(), a.1.clone()),
        (b.0, b.1),
        "same behaviour given the same results"
    );
    assert_eq!(a.0, vec!["done"]);
    assert_ne!(a.2, b.2, "only the journaled results differ");
    assert_ne!(a.3, b.3, "and the accounting");
}

trait ReplaceSummary {
    fn replace_summary(self) -> Chart;
}

impl ReplaceSummary for Chart {
    /// Structured results have no `text`: read `category` instead.
    fn replace_summary(mut self) -> Chart {
        fn walk(states: &mut [vertep_core::ir::State]) {
            for s in states {
                for t in &mut s.transitions {
                    for e in &mut t.body {
                        if let vertep_core::ir::ExecKind::Assign { location, expr, .. } =
                            &mut e.kind
                            && location == "summary"
                        {
                            *location = "category".into();
                            *expr = Some("ev.data.category".into());
                        }
                    }
                }
                walk(&mut s.children);
            }
        }
        walk(&mut self.states);
        self
    }
}

// @tg: verifies:: [[openspec:llm-actors#Provider Contract#Provider error surfaces as event]] {level: integration}
#[test]
fn provider_error_surfaces_as_event() {
    let p = ScriptedProvider::new();
    p.push(Err(ProviderError::rate_limited(Some(1_500))));
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(p)).build();
    // capture the retry-after hint too
    let chart = cel(
        "idle",
        LLM_HEAD,
        &llm_body(SUMMARISE).replace(
            r#"<assign location="class" expr="ev.data['class']"/>"#,
            r#"<assign location="class" expr="ev.data['class']"/><assign location="retry" expr="ev.data.retryAfterMs"/>"#,
        ),
    );
    spawn(&mut host, chart);
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("rate-limited".into()))
    );
    assert_eq!(ctx(&mut host, "retry"), Some(Value::Number(1_500.0)));
}

// ----- Structured Output ----------------------------------------------------------------------

const CLASSIFY: &str = r#"vt:provider="p" vt:template="classify" vt:output="{ category: billing | tech | other, urgency: int }""#;

fn classify_chart() -> Chart {
    cel(
        "idle",
        r#"<datamodel><data id="route" expr="''"/><data id="class" expr="''"/><data id="paths" expr="[]"/></datamodel>"#,
        &format!(
            r#"<state id="idle"><transition event="go" target="work"/></state>
  <state id="work">
    <invoke type="vt:llm" id="classify" {CLASSIFY}/>
    <transition event="done.invoke.classify" cond="ev.data.category.tag == 'tech' &amp;&amp; ev.data.urgency &lt; 3" target="tech"/>
    <transition event="done.invoke.classify" target="other"/>
    <transition event="error.llm" target="failed"><assign location="class" expr="ev.data['class']"/><assign location="paths" expr="ev.data.paths"/></transition>
  </state>
  <state id="tech"/><state id="other"/><state id="failed"/>"#
        ),
    )
}

// @tg: verifies:: [[openspec:llm-actors#Structured Output#Valid structured output]] {level: integration}
#[test]
fn valid_structured_output() {
    let p = ScriptedProvider::new();
    // an enumeration is a union of payload-free tags, encoded as `{"tag": ..}` in JSON
    p.push_text(r#"{"category":{"tag":"tech"},"urgency":2}"#, 5, 5);
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(p.clone())).build();
    spawn(&mut host, classify_chart());
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["tech"]);
    let schema = p.requests()[0]
        .output_schema
        .clone()
        .expect("structured output requested");
    assert_eq!(schema["type"], "object");
}

// @tg: verifies:: [[openspec:llm-actors#Structured Output#Invalid structured output]] {level: integration}
#[test]
fn invalid_structured_output() {
    let p = ScriptedProvider::new();
    p.push_text(r#"{"category":{"tag":"sales"}}"#, 5, 5);
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(p)).build();
    spawn(&mut host, classify_chart());
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("schema".into()))
    );
    let Some(Value::Array(paths)) = ctx(&mut host, "paths") else {
        panic!("violated paths")
    };
    assert!(!paths.is_empty());
    // the invalid content is journaled for audit
    let (_, AgentInput::LlmResult { content, .. }) = &llm_inputs(&host)[0] else {
        panic!("the result is journaled")
    };
    assert_eq!(content, r#"{"category":{"tag":"sales"}}"#);
}

// ----- Journaled Model Results ----------------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#Journaled Model Results#Result journaled before delivery]] {level: integration}
#[test]
fn result_journaled_before_delivery() {
    let p = ScriptedProvider::new();
    p.push_text("summary", 1, 1);
    let (mut host, _) = llm_host(Arc::new(p));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    let (pos, AgentInput::LlmResult { invokeid, .. }) = &llm_inputs(&host)[0] else {
        panic!("llm.result")
    };
    assert_eq!(invokeid, "summarise");
    assert_eq!(
        host.state_at(AGENT, pos - 1).unwrap().configuration,
        vec!["work"]
    );
    assert_eq!(
        host.state_at(AGENT, *pos).unwrap().configuration,
        vec!["done"]
    );
    assert_eq!(
        AgentInput::LlmResult {
            invokeid: String::new(),
            attempt: 1,
            provider: String::new(),
            prompt_hash: String::new(),
            content: String::new(),
            usage: Default::default(),
            model: String::new(),
            latency_ms: 0,
            cost: None,
            tool_calls: Vec::new()
        }
        .kind(),
        vocab::LLM_RESULT
    );
}

// @tg: verifies:: [[openspec:llm-actors#Journaled Model Results#Replay without provider]] {level: integration}
#[test]
fn replay_without_provider() {
    let journal = Arc::new(MemoryJournal::new());
    let p = ScriptedProvider::new();
    p.push_text("summary", 1, 1);
    let (b, _) = builder();
    let mut host = b
        .journal(journal.clone())
        .provider("p", Arc::new(p))
        .build();
    spawn(&mut host, llm_chart(SUMMARISE));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    let before = host.state(AGENT).unwrap();

    let (b, _) = builder();
    let mut replayed = b.journal(journal).build(); // no provider bindings at all
    replayed.register(llm_chart(SUMMARISE)).unwrap();
    let after = replayed.state(AGENT).unwrap();
    replayed.run_until_idle();
    assert_eq!(after.configuration, before.configuration);
    assert_eq!(after.datamodel, before.datamodel);
    assert!(replayed.stats().provider_calls.is_empty());
}

// The scenario "Forked run reuses history" is proved by the fork example (tests/examples.rs).

// ----- Retries Expressed in the Chart -------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#Retries Expressed in the Chart#Retry with backoff]] {level: integration}
#[test]
fn retry_with_backoff() {
    let p = ScriptedProvider::new();
    p.push(Err(ProviderError::rate_limited(None)));
    p.push(Err(ProviderError::rate_limited(None)));
    p.push_text("third time lucky", 1, 1);
    let (b, clock) = builder();
    let mut host = b.provider("p", Arc::new(p.clone())).build();
    spawn(
        &mut host,
        llm_chart(&format!(r#"{SUMMARISE} vt:retry="3" vt:backoff="1s""#)),
    );
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(p.calls(), 1);
    advance(&mut host, &clock, Duration::from_millis(999));
    assert_eq!(p.calls(), 1, "the first backoff is 1s");
    advance(&mut host, &clock, Duration::from_millis(1));
    assert_eq!(p.calls(), 2);
    advance(&mut host, &clock, Duration::from_millis(1_999));
    assert_eq!(p.calls(), 2, "the second backoff doubles");
    advance(&mut host, &clock, Duration::from_millis(1));
    assert_eq!(p.calls(), 3);
    assert_eq!(config(&mut host), vec!["done"]);
    let attempts: Vec<u32> = llm_inputs(&host)
        .iter()
        .map(|(_, i)| match i {
            AgentInput::LlmResult { attempt, .. } | AgentInput::LlmError { attempt, .. } => {
                *attempt
            }
            _ => 0,
        })
        .collect();
    assert_eq!(
        attempts,
        vec![1, 2, 3],
        "each attempt is a separate journaled call"
    );
    // the backoff timers were durable timers, journaled when they fired
    let timers = host
        .journal(AGENT)
        .iter()
        .filter(|e| matches!(e.input, AgentInput::Timer { .. }))
        .count();
    assert_eq!(timers, 2);
}

// @tg: verifies:: [[openspec:llm-actors#Retries Expressed in the Chart#Retries exhausted]] {level: integration}
#[test]
fn retries_exhausted() {
    let p = ScriptedProvider::new()
        .with_default(Err(ProviderError::new(ErrorClass::Transport, "down")));
    p.push(Err(ProviderError::rate_limited(None)));
    let (b, clock) = builder();
    let mut host = b.provider("p", Arc::new(p.clone())).build();
    spawn(
        &mut host,
        llm_chart(&format!(r#"{SUMMARISE} vt:retry="3" vt:backoff="10ms""#)),
    );
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    advance(&mut host, &clock, Duration::from_secs(1));
    advance(&mut host, &clock, Duration::from_secs(1));
    assert_eq!(p.calls(), 3);
    assert_eq!(config(&mut host), vec!["failed"]);
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("transport".into()))
    );
    assert_eq!(ctx(&mut host, "attempts"), Some(Value::Number(3.0)));
}

// ----- Streaming Partials -----------------------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#Streaming Partials#Partials update the context]] {level: integration}
#[test]
fn partials_update_the_context() {
    let p = ScriptedProvider::new().with_chunks(["Ada ", "wants ", "a refund."]);
    p.push_text("Ada wants a refund.", 3, 3);
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(p)).build();
    spawn(
        &mut host,
        llm_chart(&format!(r#"{SUMMARISE} vt:stream="true""#)),
    );
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert_eq!(host.stats().partials, 3);
    assert_eq!(
        ctx(&mut host, "buffer"),
        Some(Value::String("Ada wants a refund.".into()))
    );
    assert_eq!(
        ctx(&mut host, "summary"),
        Some(Value::String("Ada wants a refund.".into()))
    );
    let results = llm_inputs(&host);
    assert_eq!(results.len(), 1, "one llm.result");
    assert!(
        host.journal(AGENT).iter().all(|e| match &e.input {
            AgentInput::Event { event, .. } => event.name != vocab::LLM_PARTIAL_EVENT,
            _ => true,
        }),
        "partials are not journaled"
    );
}

// @tg: verifies:: [[openspec:llm-actors#Streaming Partials#Partial transition with a target rejected]] {level: integration}
#[test]
fn partial_transition_with_a_target_rejected() {
    let chart = cel(
        "s",
        "",
        r#"<state id="s"><transition event="vt.llm.partial" target="t"/></state><state id="t"/>"#,
    );
    assert_eq!(
        AgentChart::new(chart).unwrap_err(),
        ChartError::PartialWithTarget { state: "s".into() }
    );
}

// ----- Tool Actors --------------------------------------------------------------------------------

const SEARCH: &str = "https://example.org/tools/search";
const FETCH: &str = "https://example.org/tools/fetch";

fn search_tool(calls: Arc<Mutex<u32>>) -> Tool {
    Tool::new(
        SEARCH,
        "type Hit = { title: string };",
        "{ query: string }",
        "list<Hit>",
        move |args: Value| {
            *calls.lock().unwrap() += 1;
            async move {
                let q = vertep_agent::json::field(&args, "query").cloned();
                let Some(Value::String(q)) = q else {
                    return Err("no query".to_owned());
                };
                Ok(val(serde_json::json!([{ "title": format!("about {q}") }])))
            }
        },
    )
    .unwrap()
    .with_description("Search the knowledge base")
}

fn fetch_tool() -> Tool {
    Tool::new(FETCH, "", "{ url: iri }", "string", |_| async {
        Ok(Value::String("page".into()))
    })
    .unwrap()
}

/// An agent that decides which tool to call, then sends the request to it.
fn tool_chart() -> Chart {
    cel(
        "decide",
        r#"<datamodel><data id="hits" expr="[]"/><data id="class" expr="''"/><data id="paths" expr="[]"/></datamodel>"#,
        r#"<state id="decide" vt:decide="d" vt:candidates="call_tool">
    <transition event="call_tool" target="tooling"><send event="tool.call" targetexpr="ev.data.tool"><content expr="ev.data.args"/></send></transition>
  </state>
  <state id="tooling">
    <transition event="vt:reply" target="answered"><assign location="hits" expr="ev.data.result"/></transition>
    <transition event="error.tool" target="answered"><assign location="class" expr="ev.data['class']"/><assign location="paths" expr="ev.data.paths"/></transition>
  </state>
  <state id="answered"/>"#,
    )
}

// @tg: verifies:: [[openspec:llm-actors#Tool Actors#Tool call from an LLM decision]] {level: integration}
#[test]
fn tool_call_from_an_llm_decision() {
    let model = ScriptedProvider::new();
    model.push_text(
        &format!(r#"{{"event":"call_tool","payload":{{"tool":"{SEARCH}","args":{{"query":"refunds"}}}}}}"#),
        20,
        10,
    );
    let calls = Arc::new(Mutex::new(0));
    let (b, _) = builder();
    let mut host = b
        .decider(
            "d",
            Arc::new(vertep_agent::LlmDecider::new("p", Arc::new(model), "m")),
        )
        .tool(search_tool(calls.clone()))
        .build();
    spawn(&mut host, tool_chart());
    assert_eq!(config(&mut host), vec!["answered"]);
    assert_eq!(*calls.lock().unwrap(), 1);
    assert_eq!(
        ctx(&mut host, "hits"),
        Some(val(serde_json::json!([{"title": "about refunds"}])))
    );
    let j = host.journal(AGENT);
    assert!(
        j.iter()
            .any(|e| matches!(&e.input, AgentInput::ToolResult { tool, .. } if tool == SEARCH))
    );
}

// @tg: verifies:: [[openspec:llm-actors#Tool Actors#Invalid tool arguments]] {level: integration}
#[test]
fn invalid_tool_arguments() {
    let calls = Arc::new(Mutex::new(0));
    let rec = Recording::default();
    rec.answer(vertep_agent::DecisionOutcome::Decision(
        vertep_agent::Decision::new(
            "call_tool",
            val(serde_json::json!({"tool": SEARCH, "args": {"query": 42}})),
        ),
    ));
    let (b, _) = builder();
    let mut host = b
        .decider("d", Arc::new(rec))
        .tool(search_tool(calls.clone()))
        .build();
    spawn(&mut host, tool_chart());
    assert_eq!(config(&mut host), vec!["answered"]);
    assert_eq!(*calls.lock().unwrap(), 0, "the tool is not executed");
    assert_eq!(
        ctx(&mut host, "class"),
        Some(Value::String("arguments".into()))
    );
    assert_eq!(
        ctx(&mut host, "paths"),
        Some(val(serde_json::json!(["/query"])))
    );
}

// @tg: verifies:: [[openspec:llm-actors#Tool Actors#Tool schema offered to the model]] {level: integration}
#[test]
fn tool_schema_offered_to_the_model() {
    let p = ScriptedProvider::new();
    p.push_text("done", 1, 1);
    let (b, _) = builder();
    let mut host = b
        .provider("p", Arc::new(p.clone()))
        .tool(search_tool(Arc::new(Mutex::new(0))))
        .tool(fetch_tool())
        .tool(
            Tool::new(
                "https://example.org/tools/delete",
                "",
                "{}",
                "bool",
                |_| async { Ok(Value::Bool(true)) },
            )
            .unwrap(),
        )
        .build();
    spawn(
        &mut host,
        llm_chart(&format!(
            r##"{SUMMARISE} vt:grantTools="{SEARCH} {FETCH}" vt:grantTargets="#_parent""##
        )),
    );
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    let tools = &p.requests()[0].tools;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec![FETCH, SEARCH], "exactly the granted tools");
    let search = tools.iter().find(|t| t.name == SEARCH).unwrap();
    assert_eq!(search.input_schema["properties"]["query"]["type"], "string");
    assert_eq!(search.description, "Search the knowledge base");
}

// ----- Capability Grants (in the runtime send path) ------------------------------------------------

/// A runtime node whose `vt:assistant` invoke kind is a function actor restricted by `grant`;
/// on its first input it sends to the search tool actor, the payments actor and its parent.
fn granted_node(
    grant: Grant,
) -> (
    vertep_runtime::Node,
    vertep_runtime::Address,
    vertep_runtime::Address,
    vertep_runtime::Address,
    Arc<Mutex<Vec<grant::Denied>>>,
) {
    use vertep_runtime::actor::{
        ActorFailure, ActorStart, FnActor, OutTarget, Outgoing, TurnOutput,
    };
    use vertep_runtime::{Address, Node, Shell, VirtualClock};
    let clock = VirtualClock::new(T0);
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock),
    };
    let mut node = Node::builder(Arc::new(vertep_store::MemoryStore::new()), shell)
        .datamodels(Arc::new(vertep_dm_ecma::EcmaRegistry))
        .build();
    let sink = |name: &str| {
        chart(&format!(
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="s" name="{name}"><state id="s"><transition event="*" target="got"/></state><state id="got"/></scxml>"#
        ))
    };
    node.register_type("https://example.org/charts/search", sink("search"))
        .unwrap();
    node.register_type("https://example.org/charts/Payments", sink("payments"))
        .unwrap();
    let search = Address::virtual_actor("https://example.org/charts/search", "1").unwrap();
    let payments = Address::virtual_actor("https://example.org/charts/Payments", "1").unwrap();
    let (s2, p2) = (search.clone(), payments.clone());
    let inner: vertep_runtime::actor::ActorFactory = Arc::new(move |start: &ActorStart| {
        let (s, p) = (s2.clone(), p2.clone());
        Ok(Box::new(FnActor::new(
            start.address.clone(),
            Arc::new(move |input: vertep_runtime::actor::ActorInput| {
                let (s, p) = (s.clone(), p.clone());
                let first = input.event.name == "begin";
                Box::pin(async move {
                    if !first {
                        return Ok(TurnOutput::default());
                    }
                    Ok::<_, ActorFailure>(TurnOutput {
                        sends: vec![
                            Outgoing::new(OutTarget::Address(s), "query", Value::Undefined),
                            Outgoing::new(OutTarget::Address(p), "pay", Value::Undefined),
                            Outgoing::new(OutTarget::Parent, "answer", Value::Undefined),
                        ],
                        ..TurnOutput::default()
                    })
                })
            }),
        )) as Box<dyn vertep_runtime::actor::Actor>)
    });
    let log = Arc::new(Mutex::new(Vec::new()));
    node.register_actor_kind(
        "https://example.org/kinds/assistant",
        GrantedActor::factory(inner, grant, log.clone()),
    );
    node.register_type(
        "https://example.org/charts/agent",
        chart(
            r##"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript" initial="work">
              <datamodel><data id="denied" expr="[]"/></datamodel>
              <state id="work">
                <invoke type="https://example.org/kinds/assistant" id="assistant"/>
                <onentry><send event="begin" target="#_assistant"/></onentry>
                <transition event="vt.grant.denied"><assign location="denied" expr="denied.concat([_event.data.target])"/></transition>
                <transition event="answer" target="answered"/>
              </state>
              <state id="answered"/></scxml>"##,
        ),
    )
    .unwrap();
    let agent = Address::virtual_actor("https://example.org/charts/agent", "1").unwrap();
    (node, agent, search, payments, log)
}

fn search_address() -> vertep_runtime::Address {
    vertep_runtime::Address::virtual_actor("https://example.org/charts/search", "1").unwrap()
}

async fn run_granted() -> (
    vertep_runtime::Node,
    vertep_runtime::Address,
    vertep_runtime::Address,
    vertep_runtime::Address,
    Arc<Mutex<Vec<grant::Denied>>>,
) {
    let g = Grant::new([grant::PARENT], [search_address().as_str()]);
    let (mut node, agent, search, payments, log) = granted_node(g);
    node.send(&agent, ev("wake")).unwrap();
    for _ in 0..5 {
        node.settle().await;
    }
    (node, agent, search, payments, log)
}

// @tg: verifies:: [[openspec:llm-actors#Capability Grants#Granted send succeeds]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn granted_send_succeeds() {
    let (mut node, agent, search, _, _) = run_granted().await;
    assert_eq!(search, search_address());
    assert_eq!(node.state(&search).unwrap().configuration, vec!["got"]);
    assert_eq!(node.state(&agent).unwrap().configuration, vec!["answered"]);
}

// @tg: verifies:: [[openspec:llm-actors#Capability Grants#Ungranted send fails]] {level: integration}
#[tokio::test(flavor = "current_thread")]
async fn ungranted_send_fails() {
    let (node, agent, _, payments, log) = run_granted().await;
    // nothing reached Payments; the attempt was refused and recorded
    assert!(
        node.journal(&payments)
            .map(|j| j.is_empty())
            .unwrap_or(true)
    );
    let denied = log.lock().unwrap().clone();
    assert_eq!(denied.len(), 1);
    assert_eq!(denied[0].target, payments.as_str());
    assert_eq!(denied[0].event, "pay");
    // the parent journaled the refused attempt as an input
    let j = node.journal(&agent).unwrap();
    assert!(j.iter().any(|e| matches!(&e.body,
        vertep_runtime::EntryBody::Input { input, .. } if input.event.name == vocab::GRANT_DENIED_EVENT)));
    // the sender itself gets error.communication with the reason not-granted
    let e = grant::not_granted_event(payments.as_str(), "pay");
    assert_eq!(e.name, vocab::COMMUNICATION_ERROR_EVENT);
    assert_eq!(
        vertep_agent::json::field(&e.data, "reason"),
        Some(&Value::String("not-granted".into()))
    );
}

// @tg: verifies:: [[openspec:llm-actors#Capability Grants#Grants do not escalate]] {level: unit}
#[test]
fn grants_do_not_escalate() {
    let g = Grant::new([grant::PARENT, "https://example.org/a"], [SEARCH, FETCH]);
    let h = Grant::new(
        [grant::PARENT, "https://example.org/b"],
        [SEARCH, "https://example.org/tools/delete"],
    );
    let child = grant::effective(Some(&g), Some(&h)).unwrap();
    assert_eq!(child, Grant::new([grant::PARENT], [SEARCH]));
    // a child that declares nothing inherits, and never widens, its parent's grant
    assert_eq!(grant::effective(Some(&g), None), Some(g.clone()));
    let grandchild = grant::effective(Some(&child), Some(&g)).unwrap();
    assert_eq!(grandchild, child);
}

// ----- Usage and Cost Accounting ----------------------------------------------------------------

// @tg: verifies:: [[openspec:llm-actors#Usage and Cost Accounting#Usage recorded per call]] {level: integration}
#[test]
fn usage_recorded_per_call() {
    let p = ScriptedProvider::new();
    let mut r = text("summary", "claude-opus-5-5", 1200, 300);
    r.cost = Some(vertep_agent::llm::Cost {
        amount: 0.0108,
        currency: "USD".into(),
    });
    p.push(Ok(r));
    let (mut host, _) = llm_host(Arc::new(p));
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    let g = host.graph(AGENT).unwrap();
    let rows = sparql(
        &g,
        &format!(
            "PREFIX a: <{}> SELECT ?c ?p ?q ?m ?l ?cost WHERE {{ ?c a a:LlmCall ; a:promptTokens ?p ; a:completionTokens ?q ; a:model ?m ; a:latencyMs ?l ; a:cost ?cost }}",
            vocab::AGENT_NS
        ),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["p"], "1200");
    assert_eq!(rows[0]["q"], "300");
    assert_eq!(rows[0]["m"], "claude-opus-5-5");
    assert_eq!(rows[0]["l"], "0");
    assert_eq!(rows[0]["cost"], "0.0108");
    assert!(vertep_agent::ontology::validate(&g).conforms());
}

// @tg: verifies:: [[openspec:llm-actors#Usage and Cost Accounting#Budget guard on cumulative usage]] {level: integration}
#[test]
fn budget_guard_on_cumulative_usage() {
    let p = ScriptedProvider::new().with_default(Ok(text("more", "m", 400, 200)));
    let (b, _) = builder();
    let mut host = b.provider("p", Arc::new(p.clone())).build();
    spawn(
        &mut host,
        cel(
            "call",
            r#"<datamodel><data id="budget" expr="1000"/></datamodel>"#,
            r#"<state id="call">
                 <invoke type="vt:llm" id="step" vt:provider="p" vt:template="next"/>
                 <transition event="done.invoke.step" target="check"/>
               </state>
               <state id="check">
                 <transition cond="ctx.vt_usage.total_tokens > ctx.budget" target="budget_exhausted"/>
                 <transition target="call"/>
               </state>
               <state id="budget_exhausted"/>"#,
        ),
    );
    assert_eq!(config(&mut host), vec!["budget_exhausted"]);
    assert_eq!(p.calls(), 2, "routed out before the third call");
    let u = host.usage(AGENT).unwrap();
    assert_eq!(
        (u.prompt_tokens, u.completion_tokens, u.calls),
        (800, 400, 2)
    );
    // the system variable is read-only: an assignment to it fails with error.execution
    let (b, _) = builder();
    let mut host = b.build();
    spawn(
        &mut host,
        cel(
            "s",
            r#"<datamodel><data id="err" expr="''"/></datamodel>"#,
            r#"<state id="s">
                 <transition event="cheat"><assign location="vt_usage" expr="{'total_tokens': 0}"/></transition>
                 <transition event="error.execution"><assign location="err" expr="'refused'"/></transition>
               </state>"#,
        ),
    );
    host.send(AGENT, ev("cheat"));
    host.run_until_idle();
    assert_eq!(ctx(&mut host, "err"), Some(Value::String("refused".into())));
}

// ----- Agent Ontology Module -----------------------------------------------------------------

/// Runs a SPARQL SELECT over `g`; each row maps variables to the lexical form of their values.
fn sparql(g: &oxrdf::Graph, query: &str) -> Vec<std::collections::BTreeMap<String, String>> {
    use spareval::{QueryEvaluator, QueryResults};
    let mut ds = oxrdf::Dataset::new();
    for t in g.iter() {
        ds.insert(t.in_graph(oxrdf::GraphNameRef::DefaultGraph));
    }
    let q = spargebra::SparqlParser::new().parse_query(query).unwrap();
    let QueryResults::Solutions(sols) = QueryEvaluator::new().prepare(&q).execute(&ds).unwrap()
    else {
        panic!("a SELECT")
    };
    sols.map(|s| {
        let s = s.unwrap();
        s.iter()
            .map(|(v, t)| {
                let text = match t {
                    oxrdf::Term::Literal(l) => l.value().to_owned(),
                    oxrdf::Term::NamedNode(n) => n.as_str().to_owned(),
                    other => other.to_string(),
                };
                (v.as_str().to_owned(), text)
            })
            .collect()
    })
    .collect()
}

fn agent_run_graph() -> oxrdf::Graph {
    let p = ScriptedProvider::new();
    p.push_text("summary", 10, 2);
    let model = ScriptedProvider::new();
    model.push_text(
        r#"{"event":"go","payload":null,"rationale":"time to work","confidence":0.5}"#,
        5,
        1,
    );
    let (b, _) = builder();
    let mut host = b
        .provider("p", Arc::new(p))
        .decider(
            "d",
            Arc::new(vertep_agent::LlmDecider::new("p", Arc::new(model), "m")),
        )
        .build();
    let chart = llm_chart(SUMMARISE);
    let xml = vertep_scxml::write(&chart)
        .unwrap()
        .replace(r#"<state id="idle">"#, r#"<state id="idle" vt:decide="d">"#);
    spawn(&mut host, common::chart(&xml));
    host.run_until_idle();
    assert_eq!(config(&mut host), vec!["done"]);
    host.graph(AGENT).unwrap()
}

// @tg: verifies:: [[openspec:llm-actors#Agent Ontology Module#Shapes validate agent records]] {level: integration}
#[test]
fn shapes_validate_agent_records() {
    let g = agent_run_graph();
    let report = vertep_agent::ontology::validate(&g);
    assert!(report.conforms(), "{report:?}");
    // drop the model identifier of the LLM call
    let model = oxrdf::NamedNode::new_unchecked(vocab::MODEL);
    let llm_call = oxrdf::NamedNode::new_unchecked(vocab::LLM_CALL);
    let call = g
        .subjects_for_predicate_object(oxrdf::vocab::rdf::TYPE, &llm_call)
        .next()
        .unwrap()
        .into_owned();
    let mut broken = g.clone();
    let t = broken
        .triples_for_subject(&call)
        .find(|t| t.predicate == model.as_ref())
        .unwrap()
        .into_owned();
    broken.remove(&t);
    let report = vertep_agent::ontology::validate(&broken);
    assert!(!report.conforms());
    assert!(
        report
            .results
            .iter()
            .any(|r| r.to_string().contains("model")),
        "{report:?}"
    );
}

// @tg: verifies:: [[openspec:llm-actors#Agent Ontology Module#PROV query without reasoning]] {level: integration}
#[test]
fn prov_query_without_reasoning() {
    let g = agent_run_graph();
    let rows = sparql(
        &g,
        &format!(
            "PREFIX prov: <http://www.w3.org/ns/prov#> SELECT ?act ?type WHERE {{ ?act a prov:Activity ; prov:wasAssociatedWith <{AGENT}> . ?act a ?type . FILTER(STRSTARTS(STR(?type), \"{}\")) }} ORDER BY ?type",
            vocab::AGENT_NS
        ),
    );
    let types: Vec<&str> = rows.iter().map(|r| r["type"].as_str()).collect();
    // the decision, the model call behind it (an LLM decider) and the summarise call
    assert_eq!(
        types,
        vec![vocab::DECISION, vocab::LLM_CALL, vocab::LLM_CALL]
    );
    // the ontology itself carries the alignment axioms as plain triples
    let o = vertep_agent::ontology::agent_ontology();
    let sub = oxrdf::NamedNode::new_unchecked("http://www.w3.org/2000/01/rdf-schema#subClassOf");
    for (c, p) in [
        (vocab::LLM_CALL, vocab::PROV_ACTIVITY),
        (vocab::DECISION, vocab::PROV_ACTIVITY),
        (vocab::DECIDER, vocab::PROV_AGENT),
        (vocab::PLAN, vocab::PROV_ENTITY),
    ] {
        assert!(
            o.contains(oxrdf::TripleRef::new(
                &oxrdf::NamedNode::new_unchecked(c),
                &sub,
                &oxrdf::NamedNode::new_unchecked(p)
            )),
            "{c} ⊑ {p}"
        );
    }
    // every vt-agent: term the code writes is declared by the module
    for t in vocab::ALL {
        let n = oxrdf::NamedNode::new_unchecked(*t);
        assert!(
            o.triples_for_subject(&n).next().is_some(),
            "{t} is declared"
        );
    }
}

// @tg: verifies:: [[openspec:llm-actors#Agent Ontology Module#Plain charts need no agent module]] {level: integration}
#[test]
fn plain_charts_need_no_agent_module() {
    let plain = cel(
        "a",
        r#"<datamodel><data id="n" expr="0"/></datamodel>"#,
        r#"<state id="a"><transition event="go" target="b"><assign location="n" expr="ctx.n + 1"/></transition></state><final id="b"/>"#,
    );
    let (_, g) = vertep_agent::decide::project(&plain);
    assert!(!vertep_agent::ontology::uses_agent_terms(&g));
    let (b, _) = builder();
    let mut host = b.build();
    spawn(&mut host, plain);
    host.send(AGENT, ev("go"));
    host.run_until_idle();
    assert!(host.graph(AGENT).unwrap().is_empty());
}
