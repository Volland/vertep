//! Shared helpers for the agent integration tests.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use vertep_agent::decider::{Decider, DeciderKind, DecisionOutcome, DecisionRequest};
use vertep_agent::host::{AgentHost, AgentHostBuilder};
use vertep_core::datamodel::{Event, EventType, Value};
use vertep_core::ir::Chart;
use vertep_runtime::VirtualClock;

/// The start of the virtual clock: 2026-11-11T00:00:00Z.
pub const T0: i64 = 1_794_355_200_000;

/// The agent actor IRI used by most tests.
pub const AGENT: &str = "https://example.org/agents/a1";

/// Reads an SCXML document.
pub fn chart(xml: &str) -> Chart {
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&vertep_scxml::DeferResolver);
    vertep_scxml::read(xml, &opts)
        .unwrap_or_else(|e| panic!("chart reads: {e:?}"))
        .chart
}

/// An SCXML document with the CEL datamodel; `head` goes before the states (types, datamodel).
pub fn cel(initial: &str, head: &str, body: &str) -> Chart {
    chart(&format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="{initial}">{head}{body}</scxml>"#
    ))
}

/// A host builder on a virtual clock at [`T0`].
pub fn builder() -> (AgentHostBuilder, VirtualClock) {
    let clock = VirtualClock::new(T0);
    (AgentHost::builder(Arc::new(clock.clone())), clock)
}

/// Registers `chart` and spawns [`AGENT`] under it.
pub fn spawn(host: &mut AgentHost, chart: Chart) {
    let v = host.register(chart).expect("the chart is an agent chart");
    host.spawn(AGENT, &v).expect("the actor starts");
    host.run_until_idle();
}

/// An external event.
pub fn ev(name: &str) -> Event {
    Event::named(name, EventType::External)
}

/// An external event with data.
pub fn ev_data(name: &str, data: Value) -> Event {
    Event {
        name: name.into(),
        data,
        ..Event::default()
    }
}

/// Advances the clock and runs the host until idle.
pub fn advance(host: &mut AgentHost, clock: &VirtualClock, d: Duration) {
    clock.advance(d);
    host.run_until_idle();
}

/// The active configuration of [`AGENT`].
pub fn config(host: &mut AgentHost) -> Vec<String> {
    host.state(AGENT)
        .expect("the actor has a state")
        .configuration
}

/// The CEL context field `name` of [`AGENT`].
pub fn ctx(host: &mut AgentHost, name: &str) -> Option<Value> {
    let dm = host.state(AGENT).expect("state").datamodel;
    vertep_agent::decider::ContextView::new(dm)
        .get(name)
        .cloned()
}

/// A JSON value as a datamodel value.
pub fn val(j: serde_json::Value) -> Value {
    vertep_agent::json::from_json(&j)
}

/// A decider that records every request and cancellation and never answers by itself;
/// answers are delivered by the test through the host.
#[derive(Clone, Default)]
pub struct Recording {
    /// Requests received.
    pub requests: Arc<Mutex<Vec<DecisionRequest>>>,
    /// Cancelled correlation ids.
    pub cancelled: Arc<Mutex<Vec<String>>>,
    /// Scripted answers, consumed in order; when empty the decider never answers.
    pub answers: Arc<Mutex<std::collections::VecDeque<DecisionOutcome>>>,
}

impl Recording {
    /// The requests so far.
    pub fn requests(&self) -> Vec<DecisionRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// The cancellations so far.
    pub fn cancelled(&self) -> Vec<String> {
        self.cancelled.lock().unwrap().clone()
    }

    /// Queues an answer.
    pub fn answer(&self, o: DecisionOutcome) -> &Self {
        self.answers.lock().unwrap().push_back(o);
        self
    }

    /// The candidate event names of request `i`.
    pub fn offered(&self, i: usize) -> Vec<String> {
        self.requests()[i]
            .candidates
            .iter()
            .map(|c| c.event.clone())
            .collect()
    }
}

impl Decider for Recording {
    fn kind(&self) -> DeciderKind {
        DeciderKind::Custom
    }

    fn decide(&self, request: DecisionRequest) -> BoxFuture<'static, DecisionOutcome> {
        self.requests.lock().unwrap().push(request);
        let next = self.answers.lock().unwrap().pop_front();
        Box::pin(async move {
            match next {
                Some(o) => o,
                None => futures::future::pending().await,
            }
        })
    }

    fn cancel(&self, correlation: &str) {
        self.cancelled.lock().unwrap().push(correlation.to_owned());
    }
}

type Answer = Result<vertep_agent::llm::ProviderResult, vertep_agent::llm::ProviderError>;
type Waiting = Vec<(
    vertep_agent::llm::ProviderRequest,
    tokio::sync::oneshot::Sender<Answer>,
)>;

/// A provider whose calls stay in flight until the test releases them.
#[derive(Clone, Default)]
pub struct Gate {
    waiting: Arc<Mutex<Waiting>>,
}

impl Gate {
    /// The calls in flight.
    pub fn in_flight(&self) -> usize {
        self.waiting.lock().unwrap().len()
    }

    /// Answers the oldest call in flight; false when none is (or it was cancelled).
    pub fn release(
        &self,
        r: Result<vertep_agent::llm::ProviderResult, vertep_agent::llm::ProviderError>,
    ) -> bool {
        let mut w = self.waiting.lock().unwrap();
        if w.is_empty() {
            return false;
        }
        let (_, tx) = w.remove(0);
        tx.send(r).is_ok()
    }
}

impl vertep_agent::llm::Provider for Gate {
    fn complete(
        &self,
        request: vertep_agent::llm::ProviderRequest,
    ) -> BoxFuture<
        'static,
        Result<vertep_agent::llm::ProviderResult, vertep_agent::llm::ProviderError>,
    > {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.waiting.lock().unwrap().push((request, tx));
        Box::pin(async move {
            rx.await.unwrap_or_else(|_| {
                Err(vertep_agent::llm::ProviderError::new(
                    vertep_agent::llm::ErrorClass::Transport,
                    "cancelled",
                ))
            })
        })
    }
}

/// A text result with usage.
pub fn text(
    content: &str,
    model: &str,
    prompt: u64,
    completion: u64,
) -> vertep_agent::llm::ProviderResult {
    vertep_agent::llm::ProviderResult::text(
        content,
        model,
        vertep_agent::llm::Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
        },
    )
}
