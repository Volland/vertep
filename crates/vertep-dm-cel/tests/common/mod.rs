//! IR builders and a session runner for the CEL datamodel scenario tests.
#![allow(dead_code)]

use std::collections::BTreeSet;

use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Environment, Event, EventType, SessionInfo, StateQuery, Value};
use vertep_core::ir::{
    Block, Chart, Content, Data, DoneData, Exec, ExecKind, IfBranch, Param, Span, State,
    StateElement, Transition,
};
use vertep_core::session::{Input, Options, Session, StartInput, start, step};
use vertep_core::trace::MacrostepTrace;
use vertep_dm_cel::CelDatamodel;

pub fn atomic(id: &str) -> State {
    State::new(id, StateElement::State)
}

pub fn compound(id: &str, initial: &str, children: Vec<State>) -> State {
    let mut s = State::new(id, StateElement::State);
    s.initial = vec![initial.to_string()];
    s.children = children;
    s
}

pub fn final_(id: &str) -> State {
    State::new(id, StateElement::Final)
}

pub fn tr(events: &str, target: &str) -> Transition {
    Transition {
        events: events.split_whitespace().map(String::from).collect(),
        targets: target.split_whitespace().map(String::from).collect(),
        ..Transition::default()
    }
}

pub fn when(mut t: Transition, cond: &str) -> Transition {
    t.cond = Some(cond.to_string());
    t
}

pub fn then(mut t: Transition, body: Block) -> Transition {
    t.body = body;
    t
}

pub fn on(mut s: State, t: Transition) -> State {
    s.transitions.push(t);
    s
}

pub fn entry(mut s: State, b: Block) -> State {
    s.onentry.push(b);
    s
}

pub fn with_data(mut s: State, d: Vec<Data>) -> State {
    s.data = d;
    s
}

pub fn data(id: &str, expr: &str) -> Data {
    Data {
        id: id.to_string(),
        src: None,
        expr: Some(expr.to_string()),
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    }
}

pub fn assign(location: &str, expr: &str) -> Exec {
    Exec::new(ExecKind::Assign {
        location: location.to_string(),
        expr: Some(expr.to_string()),
        content: None,
    })
}

pub fn log(expr: &str) -> Exec {
    Exec::new(ExecKind::Log {
        label: None,
        expr: Some(expr.to_string()),
    })
}

pub fn script(body: &str) -> Exec {
    Exec::new(ExecKind::Script(vertep_core::ir::Script {
        src: None,
        body: Some(body.to_string()),
        span: Span::NONE,
    }))
}

pub fn if_(cond: &str, body: Block) -> Exec {
    Exec::new(ExecKind::If {
        branches: vec![IfBranch {
            cond: Some(cond.to_string()),
            body,
            span: Span::NONE,
        }],
    })
}

pub fn foreach(array: &str, item: &str, index: Option<&str>, body: Block) -> Exec {
    Exec::new(ExecKind::Foreach {
        array: array.to_string(),
        item: item.to_string(),
        index: index.map(String::from),
        body,
    })
}

pub fn donedata(mut s: State, params: &[(&str, &str)]) -> State {
    s.donedata = Some(DoneData {
        content: None::<Content>,
        params: params
            .iter()
            .map(|(n, e)| Param {
                name: n.to_string(),
                expr: Some(e.to_string()),
                location: None,
                span: Span::NONE,
            })
            .collect(),
        span: Span::NONE,
    });
    s
}

/// A CEL chart with root data and top-level states; the first state is initial.
pub fn chart(data: Vec<Data>, states: Vec<State>) -> Chart {
    Chart {
        datamodel: "cel".to_string(),
        datamodel_explicit: true,
        data,
        states,
        ..Chart::default()
    }
}

pub fn event(name: &str, data: Value) -> Event {
    Event {
        name: name.to_string(),
        type_: EventType::External,
        data,
        ..Event::default()
    }
}

pub fn obj(pairs: &[(&str, Value)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

/// A running session over a CEL datamodel.
pub struct Run {
    pub chart: CompiledChart,
    pub session: Session,
    pub dm: CelDatamodel,
    pub traces: Vec<MacrostepTrace>,
}

impl Run {
    pub fn start(chart: Chart, mut dm: CelDatamodel) -> Run {
        let chart = CompiledChart::new(chart).expect("valid chart");
        let input = StartInput {
            info: SessionInfo {
                sessionid: "s1".to_string(),
                name: Some("test".to_string()),
                ioprocessors: Value::Undefined,
            },
            env: Environment::default(),
            options: Options::default(),
            ..StartInput::default()
        };
        let out = start(&chart, input, &mut dm).expect("session starts");
        Run {
            chart,
            session: out.session,
            dm,
            traces: vec![out.trace],
        }
    }

    pub fn send(&mut self, name: &str, data: Value) {
        let input = Input::event(event(name, data), Environment::default());
        let out = step(&self.chart, self.session.clone(), input, &mut self.dm).expect("step");
        self.session = out.session;
        self.traces.push(out.trace);
    }

    pub fn configuration(&self) -> Vec<String> {
        self.traces
            .last()
            .map(|t| t.configuration.clone())
            .unwrap_or_default()
    }

    /// Every event consumed in the last macrostep.
    pub fn last_events(&self) -> Vec<String> {
        self.traces
            .last()
            .map(|t| {
                t.microsteps
                    .iter()
                    .filter_map(|m| m.event.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A fixed active configuration for `In()`.
pub struct Active(pub BTreeSet<String>);

impl Active {
    pub fn of(ids: &[&str]) -> Active {
        Active(ids.iter().map(|s| s.to_string()).collect())
    }
}

impl StateQuery for Active {
    fn in_state(&self, id: &str) -> bool {
        self.0.contains(id)
    }
}
