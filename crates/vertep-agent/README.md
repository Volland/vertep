<p align="center">
  <a href="https://volland.github.io/vertep/"><img src="https://volland.github.io/vertep/assets/img/logo.svg" alt="Vertep" width="120"></a>
</p>

<h1 align="center">vertep-agent</h1>

<p align="center">Deterministic agents on Vertep charts: deciders at <code>vt:decide</code> states, LLM and tool actors, grants and plan charts</p>

<p align="center">
  <a href="https://crates.io/crates/vertep-agent"><img src="https://img.shields.io/crates/v/vertep-agent.svg" alt="crates.io"></a>
  <a href="https://docs.rs/vertep-agent"><img src="https://img.shields.io/docsrs/vertep-agent" alt="docs.rs"></a>
  <a href="https://github.com/Volland/vertep/blob/main/LICENSE-MIT"><img src="https://img.shields.io/crates/l/vertep-agent.svg" alt="license"></a>
</p>

In Vertep an agent is an ordinary SCXML chart. At a state marked `vt:decide`, a decider (a
language model, a person, a rule table or another actor) picks the next event, but it is
offered only the events the chart enables at that moment. The answer is written to the journal
before the chart sees it, and checked before it is taken. So a run replays without calling any
model, and a fork from any journal position explores another choice without editing history.

The crate adds no execution semantics to SCXML: decide states, `vt:llm` and `vt:tool` invokes,
grants and plans all lower onto states, `<invoke>`, `<send>`, internal events, durable timers
and journal entries. It builds on `vertep-core` (the pure step and `enabled_events`),
`vertep-runtime` (clocks, nodes, addresses), `vertep-dm-cel` (the default datamodel) and
`vertep-rdf` (the `vt-agent:` ontology). The `vertep` facade re-exports it as `vertep::agent`
when you turn on its `agent` feature; depend on this crate directly if you do not use the
facade.

## Install

```toml
[dependencies]
vertep-agent = "0.1"
vertep-core = "0.1"     # Event, Value, Chart
vertep-scxml = "0.1"    # read SCXML charts
vertep-runtime = "0.1"  # VirtualClock and the system clock
serde_json = "1"        # the example builds an event payload from JSON
# or, through the facade:
# vertep = { version = "0.1", features = ["agent"] }
```

| Feature | Turns on |
| --- | --- |
| `anthropic` | `anthropic::AnthropicProvider`: request and response mapping for the Anthropic Messages API, over an `anthropic::HttpClient` your application supplies. No HTTP stack is pulled in |

## Example

This is the [decider-agent](https://volland.github.io/vertep/examples/decider-agent.html)
example, shortened. A scripted decider stands in for an LLM, so it runs with no network and no
API key.

```rust
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use vertep_agent::{
    AgentHost, Decider, DeciderKind, Decision, DecisionOutcome, DecisionRequest, MemoryJournal,
};
use vertep_core::datamodel::{Event, Value};
use vertep_runtime::VirtualClock;

const REFUND: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
       version="1.0" datamodel="cel" initial="idle">
  <datamodel><data id="amount" expr="0"/></datamodel>
  <state id="idle">
    <transition event="request" target="review"><assign location="amount" expr="ev.data.amount"/></transition>
  </state>
  <state id="review" vt:decide="policy" vt:candidates="approve reject escalate">
    <transition event="approve" cond="ctx.amount &lt;= 100" target="approved"/>
    <transition event="reject" target="rejected"/>
    <transition event="escalate" cond="ctx.amount > 100" target="manager"/>
  </state>
  <state id="manager" vt:decide="policy" vt:candidates="approve reject">
    <transition event="approve" target="approved"/>
    <transition event="reject" target="rejected"/>
  </state>
  <state id="approved"/>
  <state id="rejected"/>
</scxml>"#;
const AGENT: &str = "https://example.org/agents/refund/7";

/// Answers from a script and records what it was offered.
struct Scripted {
    answers: Mutex<VecDeque<&'static str>>,
    offers: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Decider for Scripted {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Custom
    }
    fn decide(&self, request: DecisionRequest) -> Pin<Box<dyn Future<Output = DecisionOutcome> + Send>> {
        let offered = request.candidates.iter().map(|c| c.event.clone()).collect();
        self.offers.lock().unwrap().push(offered);
        let outcome = match self.answers.lock().unwrap().pop_front() {
            Some(event) => DecisionOutcome::Decision(Decision::new(event, Value::Undefined)),
            None => DecisionOutcome::refuse("the script is exhausted"),
        };
        Box::pin(async move { outcome })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let chart = || -> Result<_, Box<dyn std::error::Error>> {
        let opts = vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel);
        Ok(vertep_scxml::read(REFUND, &opts)?.chart)
    };
    let journal = Arc::new(MemoryJournal::new());
    let offers = Arc::new(Mutex::new(Vec::new()));

    // Run 1: the decider bound to `policy` answers twice.
    let decider = Scripted { answers: Mutex::new(VecDeque::from(["escalate", "approve"])), offers: offers.clone() };
    let mut host = AgentHost::builder(Arc::new(VirtualClock::new(0)))
        .journal(journal.clone())
        .decider("policy", Arc::new(decider))
        .build();
    let version = host.register(chart()?)?;
    host.spawn(AGENT, &version)?;
    host.send(AGENT, Event {
        name: "request".into(),
        data: vertep_agent::json::from_json(&serde_json::json!({ "amount": 250 })),
        ..Event::default()
    });
    host.run_until_idle();
    assert_eq!(host.state(AGENT)?.configuration, ["approved"]);
    // With amount 250, `approve` was never offered in `review`: its guard is false.
    assert_eq!(*offers.lock().unwrap(), [vec!["reject", "escalate"], vec!["approve", "reject"]]);
    let kinds: Vec<_> = host.journal(AGENT).iter().map(|e| e.input.kind()).collect();
    assert_eq!(kinds, ["start", "event", "decision", "decision"]);

    // Run 2: the same journal, no decider bound. Replay gives the same state.
    let mut replay = AgentHost::builder(Arc::new(VirtualClock::new(0))).journal(journal).build();
    replay.register(chart()?)?;
    assert_eq!(replay.state(AGENT)?.configuration, ["approved"]);
    assert_eq!(replay.stats().decider_calls.values().sum::<usize>(), 0);
    Ok(())
}
```

The refund chart has two decide states bound to `policy`. In `review`, `approve` has the guard
`ctx.amount <= 100`, so for a request of 250 the decider is offered only `reject` and
`escalate`, whatever it would prefer. Each answer is journaled as a `decision` entry before the
chart applies it. A second host on the same journal, with no decider at all, replays the four
entries and reaches the same state without asking anyone.

The crate's own examples run a ReAct support agent with a scripted model and two tools, and
fork its journal to try another ending:

```sh
cargo run -p vertep-agent --example react_support_agent
cargo run -p vertep-agent --example fork_from_journal
```

## Key types

| Item | What it is |
| --- | --- |
| `AgentHost`, `AgentHostBuilder` | the deterministic shell for agent actors: `register`, `spawn`, `send`, `run_until_idle`, `state`, `state_at`, `journal`, `fork`, `usage`, `offers`, `graph` |
| `Decider`, `DeciderKind`, `DecisionRequest`, `DecisionOutcome`, `Decision` | the decider contract: a request with the candidates, a read-only `ContextView` and a correlation id; an answer or a `Refusal` |
| `GuardDecider`, `LlmDecider`, `HumanDecider` (with `Inbox`), `ActorDecider` | the built-in decider kinds: a guard table, a model, a person's inbox, another actor |
| `enabled_events`, `Candidate`, `DecideSpec` | the candidate set of a decide state, from the pure core; the parsed `vt:decide` binding |
| `Provider`, `ProviderRequest`, `ProviderResult`, `ScriptedProvider` | the LLM provider contract behind `<invoke type="vt:llm">` and `LlmDecider`; a scripted provider for tests |
| `Tool`, `ToolRegistry`, `Grant` | tools with typed arguments and results; capability grants enforced in the send path |
| `plan` (`AcceptedPlan`, `PlanBounds`, `PlanError`) | LLM-written plan charts: parsed, validated, content-addressed, run as sandboxed children |
| `AgentMachine`, `AgentChart` | the pure agent machine the host drives from journaled inputs |
| `AgentJournal`, `MemoryJournal`, `AgentInput`, `JournalEntry` | the agent journal contract and its in-memory implementation; input kinds `start`, `event`, `decision`, `refusal`, `timer`, `llm.result`, `llm.error`, `tool.result` |
| `ontology` | the `vt-agent:` ontology module, its SHACL shapes and the provenance graph of a run |

Decide-state attributes: `vt:decide` (the binding), `vt:candidates`, `vt:deadline` (a durable
timer), `vt:fallback` and `vt:maxDecisions`.

## Known gaps

- The only `AgentJournal` in the crate is `MemoryJournal`, which keeps entries in memory. A
  durable agent journal on tiramemsu or another store is not written yet; implement
  `AgentJournal` (append with a position check, read, list actors) to get one.
- `AnthropicProvider` is the only vendor mapping, and it needs an `HttpClient` from you.
- Grants restrict invoked children only. A chart's own `<send>` goes wherever the chart says,
  and on an untyped chart a decision is checked by event name only, so give events that carry
  model-written data a payload type or a guard.

## Documentation

- Website: https://volland.github.io/vertep/ ([Agents and deciders](https://volland.github.io/vertep/docs/agents.html))
- API reference: https://docs.rs/vertep-agent
- Examples: https://volland.github.io/vertep/examples/ ([decider agent](https://volland.github.io/vertep/examples/decider-agent.html))
- Article: [Deterministic agents](https://volland.github.io/vertep/articles/deterministic-agents.html)

## License

MIT OR Apache-2.0, at your option.
