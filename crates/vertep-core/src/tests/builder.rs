//! Small IR builders and a mock datamodel for the scenario tests.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::compile::CompiledChart;
use crate::datamodel::{
    Datamodel, DatamodelRegistry, Environment, EvalError, Event, EventType, SessionInfo,
    StateQuery, Value,
};
use crate::ir::{
    Block, Chart, Content, DoneData, Exec, ExecKind, HistoryType, IfBranch, Invoke, Send, Span,
    State, StateElement, Transition, TransitionType,
};
use crate::null::NullDatamodel;
use crate::session::{Effect, Input, Options, Outcome, Session, StartInput, start, step};

pub fn atomic(id: &str) -> State {
    State::new(id, StateElement::State)
}

pub fn compound(id: &str, children: Vec<State>) -> State {
    let mut s = State::new(id, StateElement::State);
    s.children = children;
    s
}

pub fn parallel(id: &str, children: Vec<State>) -> State {
    let mut s = State::new(id, StateElement::Parallel);
    s.children = children;
    s
}

pub fn final_(id: &str) -> State {
    State::new(id, StateElement::Final)
}

pub fn history(id: &str, ty: HistoryType, default_targets: &str) -> State {
    let mut s = State::new(id, StateElement::History(ty));
    s.transitions.push(tr("", default_targets));
    s
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

/// A transition; `events` and `targets` are whitespace-separated, empty for none.
pub fn tr(events: &str, targets: &str) -> Transition {
    Transition {
        events: words(events),
        targets: words(targets),
        ..Transition::default()
    }
}

pub trait TransitionExt {
    fn cond(self, c: &str) -> Self;
    fn body(self, b: Block) -> Self;
    fn internal(self) -> Self;
}

impl TransitionExt for Transition {
    fn cond(mut self, c: &str) -> Self {
        self.cond = Some(c.to_string());
        self
    }
    fn body(mut self, b: Block) -> Self {
        self.body = b;
        self
    }
    fn internal(mut self) -> Self {
        self.kind = TransitionType::Internal;
        self.kind_explicit = true;
        self
    }
}

pub trait StateExt {
    fn on(self, t: Transition) -> Self;
    fn entry(self, b: Block) -> Self;
    fn exit(self, b: Block) -> Self;
    fn initial(self, ids: &str) -> Self;
    fn invoke(self, i: Invoke) -> Self;
    fn donedata(self, content: &str) -> Self;
    /// `onentry` logging `enter <id>` and `onexit` logging `exit <id>`.
    fn logged(self) -> Self;
}

impl StateExt for State {
    fn on(mut self, t: Transition) -> Self {
        self.transitions.push(t);
        self
    }
    fn entry(mut self, b: Block) -> Self {
        self.onentry.push(b);
        self
    }
    fn exit(mut self, b: Block) -> Self {
        self.onexit.push(b);
        self
    }
    fn initial(mut self, ids: &str) -> Self {
        self.initial = words(ids);
        self
    }
    fn invoke(mut self, i: Invoke) -> Self {
        self.invokes.push(i);
        self
    }
    fn donedata(mut self, content: &str) -> Self {
        self.donedata = Some(DoneData {
            content: Some(Content {
                expr: None,
                body: Some(content.to_string()),
                span: Span::NONE,
            }),
            params: Vec::new(),
            span: Span::NONE,
        });
        self
    }
    fn logged(self) -> Self {
        let enter = alloc::format!("enter {}", self.id);
        let exit = alloc::format!("exit {}", self.id);
        self.entry(alloc::vec![log(&enter)])
            .exit(alloc::vec![log(&exit)])
    }
}

pub fn log(label: &str) -> Exec {
    Exec::new(ExecKind::Log {
        label: Some(label.to_string()),
        expr: None,
    })
}

pub fn log_expr(expr: &str) -> Exec {
    Exec::new(ExecKind::Log {
        label: None,
        expr: Some(expr.to_string()),
    })
}

pub fn raise(event: &str) -> Exec {
    Exec::new(ExecKind::Raise {
        event: event.to_string(),
    })
}

pub fn send(s: Send) -> Exec {
    Exec::new(ExecKind::Send(Box::new(s)))
}

pub fn send_event(event: &str) -> Send {
    Send {
        event: Some(event.to_string()),
        ..Send::default()
    }
}

pub fn assign(location: &str, expr: &str) -> Exec {
    Exec::new(ExecKind::Assign {
        location: location.to_string(),
        expr: Some(expr.to_string()),
        content: None,
    })
}

pub fn if_(branches: Vec<(Option<&str>, Block)>) -> Exec {
    Exec::new(ExecKind::If {
        branches: branches
            .into_iter()
            .map(|(c, body)| IfBranch {
                cond: c.map(String::from),
                body,
                span: Span::NONE,
            })
            .collect(),
    })
}

pub fn chart(states: Vec<State>) -> Chart {
    Chart {
        states,
        ..Chart::default()
    }
}

pub fn compile(c: Chart) -> CompiledChart {
    match CompiledChart::new(c) {
        Ok(cc) => cc,
        Err(v) => panic!("chart is not well-formed: {v:?}"),
    }
}

pub fn start_input() -> StartInput {
    StartInput {
        info: SessionInfo {
            sessionid: "sid".to_string(),
            name: None,
            ioprocessors: Value::Undefined,
        },
        env: Environment::default(),
        options: Options::default(),
        data: Vec::new(),
        sessions: None,
    }
}

/// Start a session with the null datamodel.
pub fn run_null(cc: &CompiledChart) -> Outcome {
    start(cc, start_input(), &mut NullDatamodel).expect("start")
}

pub fn ev(name: &str) -> Event {
    Event::named(name, EventType::External)
}

pub fn send_null(cc: &CompiledChart, s: Session, name: &str) -> Outcome {
    step(
        cc,
        s,
        Input::event(ev(name), Environment::default()),
        &mut NullDatamodel,
    )
    .expect("step")
}

/// The labels of the `Log` effects, in order.
pub fn logs(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log { label: Some(l), .. } => Some(l.clone()),
            _ => None,
        })
        .collect()
}

/// Every event consumed in the macrostep, in order (from the trace).
pub fn events(o: &Outcome) -> Vec<String> {
    o.trace
        .microsteps
        .iter()
        .filter_map(|m| m.event.clone())
        .collect()
}

pub fn active(cc: &CompiledChart, o: &Outcome) -> Vec<String> {
    o.session.active(cc)
}

pub fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A tiny datamodel for tests: variables holding values; expressions are numeric or quoted
/// string literals, variable names, `In('id')`, `true`, `false`, `<var> == <number>`, and
/// `fail` (always an error).
#[derive(Clone, Debug, Default)]
pub struct MockDm {
    pub name: String,
    pub vars: BTreeMap<String, Value>,
    pub event: Option<Event>,
    pub calls: Vec<String>,
}

impl MockDm {
    pub fn named(name: &str) -> Self {
        MockDm {
            name: name.to_string(),
            ..MockDm::default()
        }
    }

    fn value(&self, expr: &str) -> Result<Value, EvalError> {
        let e = expr.trim();
        if e == "fail" {
            return Err(EvalError::new("fail"));
        }
        if let Ok(n) = e.parse::<f64>() {
            return Ok(Value::Number(n));
        }
        if e.len() >= 2 && e.starts_with('\'') && e.ends_with('\'') {
            return Ok(Value::String(e[1..e.len() - 1].to_string()));
        }
        self.vars
            .get(e)
            .cloned()
            .ok_or_else(|| EvalError::new(alloc::format!("undefined variable {e}")))
    }
}

impl Datamodel for MockDm {
    fn name(&self) -> &str {
        &self.name
    }
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        self.calls.push("init_session".into());
        self.vars
            .insert("_sessionid".into(), Value::String(info.sessionid.clone()));
        Ok(())
    }
    fn set_environment(&mut self, _env: &Environment) {
        self.calls.push("set_environment".into());
    }
    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        self.calls.push(alloc::format!("declare {id}"));
        self.vars.insert(id.to_string(), Value::Undefined);
        Ok(())
    }
    fn set_event(&mut self, event: &Event) {
        self.calls.push(alloc::format!("set_event {}", event.name));
        self.event = Some(event.clone());
    }
    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        self.calls.push(alloc::format!("eval_cond {expr}"));
        if let Some(id) = crate::null::parse_in_predicate(expr) {
            return Ok(states.in_state(id));
        }
        match expr.trim() {
            "true" => return Ok(true),
            "false" => return Ok(false),
            _ => {}
        }
        if let Some((l, r)) = expr.split_once("==") {
            return Ok(self.value(l)? == self.value(r)?);
        }
        Err(EvalError::new("bad condition"))
    }
    fn eval_value(&mut self, expr: &str, _states: &dyn StateQuery) -> Result<Value, EvalError> {
        self.calls.push(alloc::format!("eval_value {expr}"));
        self.value(expr)
    }
    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        Ok(Value::String(content.to_string()))
    }
    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        _states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        self.calls.push(alloc::format!("assign_expr {location}"));
        let v = self.value(expr)?;
        self.assign_value(location, v)
    }
    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        match self.vars.get_mut(location) {
            Some(slot) if !location.starts_with('_') => {
                *slot = value;
                Ok(())
            }
            _ => Err(EvalError::new("not a location")),
        }
    }
    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        self.vars.insert(name.to_string(), value);
        Ok(())
    }
    fn is_location(&mut self, location: &str) -> bool {
        self.vars.contains_key(location)
    }
    fn exec_script(&mut self, script: &str, _states: &dyn StateQuery) -> Result<(), EvalError> {
        if script.trim() == "fail" {
            Err(EvalError::new("fail"))
        } else {
            Ok(())
        }
    }
    fn foreach_items(
        &mut self,
        array: &str,
        _states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        match self.value(array)? {
            Value::Array(v) => Ok(v),
            _ => Err(EvalError::new("not an array")),
        }
    }
    fn snapshot(&self) -> Value {
        Value::Object(
            self.vars
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }
    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        match snapshot {
            Value::Object(pairs) => {
                self.vars = pairs.iter().cloned().collect();
                Ok(())
            }
            _ => Err(EvalError::new("bad snapshot")),
        }
    }
}

/// A host registry offering one mock datamodel under a name.
pub struct MockRegistry(pub &'static str);

impl DatamodelRegistry for MockRegistry {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        (name == self.0).then(|| Box::new(MockDm::named(self.0)) as Box<dyn Datamodel>)
    }
}
