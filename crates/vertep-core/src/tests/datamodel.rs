//! Tests for the datamodel requirements the core realizes: registry, `In()`, the `null`
//! datamodel, binding modes and the `null` snapshot; plus the tape.

use alloc::vec;
use alloc::vec::Vec;

use super::builder::*;
use crate::datamodel::{Datamodel, Environment, StateQuery, Value};
use crate::ir::*;
use crate::null::{NullDatamodel, NullRegistry, WithNull, create_datamodel, start_with_registry};
use crate::session::{Input, StartError, start, step};
use crate::tape::{Answer, Request, TapeDatamodel};

fn data(id: &str, expr: &str) -> Data {
    Data {
        id: id.into(),
        src: None,
        expr: Some(expr.into()),
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    }
}

// @tg: verifies:: [[openspec:datamodel#Datamodel Registry#Registered datamodel selected]] {level: unit}
#[test]
fn registered_datamodel_selected() {
    let mut c = chart(vec![
        atomic("s0").on(tr("", "pass").cond("x == 1")),
        atomic("pass"),
    ]);
    c.datamodel = "ecmascript".into();
    c.datamodel_explicit = true;
    c.data.push(data("x", "1"));
    let cc = compile(c);
    let (o, dm) = start_with_registry(&cc, &WithNull(MockRegistry("ecmascript")), start_input())
        .expect("starts");
    assert_eq!(dm.name(), "ecmascript");
    assert_eq!(active(&cc, &o), ids(&["pass"]));
    assert_eq!(
        dm.snapshot(),
        Value::Object(vec![
            ("_sessionid".into(), Value::String("sid".into())),
            ("x".into(), Value::Number(1.0)),
        ])
    );
    // A datamodel that is not the one the chart names is refused.
    assert!(matches!(
        start(&cc, start_input(), &mut NullDatamodel),
        Err(StartError::DatamodelMismatch { .. })
    ));
}

// @tg: verifies:: [[openspec:datamodel#Datamodel Registry#Missing datamodel refused]] {level: unit}
#[test]
fn missing_datamodel_refused() {
    let mut c = chart(vec![atomic("s")]);
    c.datamodel = "xpath".into();
    let cc = compile(c);
    let r = start_with_registry(&cc, &NullRegistry, start_input());
    assert!(matches!(
        r,
        Err(StartError::UnsupportedDatamodel { ref name }) if name == "xpath"
    ));
    // `null` is always available and is the default when the chart names none.
    assert_eq!(
        create_datamodel(&MockRegistry("ecmascript"), "")
            .unwrap()
            .name(),
        "null"
    );
    let cc = compile(chart(vec![atomic("s")]));
    assert_eq!(cc.datamodel(), "null");
    assert!(start_with_registry(&cc, &MockRegistry("ecmascript"), start_input()).is_ok());
}

struct Only(&'static str);
impl StateQuery for Only {
    fn in_state(&self, id: &str) -> bool {
        id == self.0
    }
}

fn in_chart() -> crate::compile::CompiledChart {
    compile(chart(vec![parallel(
        "p",
        vec![
            compound(
                "r1",
                vec![
                    atomic("a").on(tr("check", "pass").cond("In('b')")),
                    atomic("pass"),
                ],
            ),
            compound("r2", vec![atomic("c").on(tr("tob", "b")), atomic("b")]),
        ],
    )]))
}

// @tg: verifies:: [[openspec:datamodel#In Predicate#Null datamodel condition]] {level: unit}
#[test]
fn null_datamodel_condition() {
    assert_eq!(NullDatamodel.eval_cond("In('b')", &Only("b")), Ok(true));
    assert_eq!(
        NullDatamodel.eval_cond(" In( \"b\" ) ", &Only("b")),
        Ok(true)
    );
    let cc = in_chart();
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "tob");
    assert!(o.session.is_active(&cc, "b"));
    let o = send_null(&cc, o.session, "check");
    assert!(o.session.is_active(&cc, "pass"));
}

// @tg: verifies:: [[openspec:datamodel#In Predicate#State not active]] {level: unit}
#[test]
fn state_not_active() {
    assert_eq!(NullDatamodel.eval_cond("In('b')", &Only("c")), Ok(false));
    let cc = in_chart();
    let o = run_null(&cc);
    assert!(!o.session.is_active(&cc, "b"));
    let o = send_null(&cc, o.session, "check");
    assert!(o.session.is_active(&cc, "a"));
    assert!(!o.session.is_active(&cc, "pass"));
}

// @tg: verifies:: [[openspec:datamodel#Null Datamodel#Assign in a null chart]] {level: unit}
#[test]
fn assign_in_a_null_chart() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![assign("x", "1"), log("not reached")])
            .on(tr("error.execution", "pass")),
        atomic("pass"),
    ]));
    let o = run_null(&cc);
    assert_eq!(events(&o), ids(&["error.execution"]));
    assert!(logs(&o.effects).is_empty());
    assert_eq!(active(&cc, &o), ids(&["pass"]));
    // Every non-In() operation is an evaluation error.
    let mut dm = NullDatamodel;
    assert!(dm.eval_cond("true", &Only("s")).is_err());
    assert!(dm.eval_value("1", &Only("s")).is_err());
    assert!(dm.exec_script("x", &Only("s")).is_err());
    assert!(dm.foreach_items("[1]", &Only("s")).is_err());
    assert!(dm.declare("x").is_err());
}

// @tg: verifies:: [[openspec:datamodel#Null Datamodel#Pure control flow runs]] {level: unit}
#[test]
fn pure_control_flow_runs() {
    let cc = compile(chart(vec![
        atomic("s0")
            .entry(vec![raise("go"), send(send_event("ext"))])
            .on(tr("go", "s1")),
        atomic("s1").on(tr("ext", "s2").cond("In('s1')")),
        atomic("s2")
            .entry(vec![send(crate::ir::Send {
                target: Some("#_internal".into()),
                ..send_event("finish")
            })])
            .on(tr("finish", "end")),
        final_("end"),
        atomic("error").on(tr("error", "end")),
    ]));
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["s1"]));
    let crate::session::Effect::Send(req) = &o.effects[0] else {
        panic!("expected the external send")
    };
    // The host delivers the external send back to the session.
    let o = step(
        &cc,
        o.session,
        Input::event(req.event.clone(), Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert!(o.session.is_terminated());
    assert_eq!(events(&o), ids(&["ext", "finish"]));
    assert!(matches!(
        o.effects.last(),
        Some(crate::session::Effect::SessionDone {
            cancelled: false,
            ..
        })
    ));
}

fn binding_chart(binding: Binding) -> crate::compile::CompiledChart {
    let mut s = atomic("s");
    s.data.push(data("x", "1"));
    let mut c = chart(vec![atomic("s0").on(tr("go", "s")), s]);
    c.datamodel = "mock".into();
    c.binding = binding;
    compile(c)
}

// @tg: verifies:: [[openspec:datamodel#Data Binding Modes#Early binding]] {level: unit}
#[test]
fn early_binding() {
    let cc = binding_chart(Binding::Early);
    let mut dm = MockDm::named("mock");
    let o = start(&cc, start_input(), &mut dm).unwrap();
    assert!(!o.session.is_active(&cc, "s"));
    assert_eq!(dm.vars.get("x"), Some(&Value::Number(1.0)));
}

// @tg: verifies:: [[openspec:datamodel#Data Binding Modes#Late binding]] {level: unit}
#[test]
fn late_binding() {
    let cc = binding_chart(Binding::Late);
    let mut dm = MockDm::named("mock");
    let o = start(&cc, start_input(), &mut dm).unwrap();
    // Created at start, but not yet initialized.
    assert_eq!(dm.vars.get("x"), Some(&Value::Undefined));
    let o = step(
        &cc,
        o.session,
        Input::event(ev("go"), Environment::default()),
        &mut dm,
    )
    .unwrap();
    assert!(o.session.is_active(&cc, "s"));
    assert_eq!(dm.vars.get("x"), Some(&Value::Number(1.0)));
}

// @tg: verifies:: [[openspec:datamodel#Datamodel Snapshot#Snapshot of the null datamodel]] {level: unit}
#[test]
fn snapshot_of_the_null_datamodel() {
    let mut dm = NullDatamodel;
    assert_eq!(dm.snapshot(), Value::Undefined);
    assert_eq!(dm.restore(&Value::Undefined), Ok(()));
    assert!(dm.restore(&Value::Number(1.0)).is_err());
}

/// The tape records every request with its answer, in order.
#[test]
fn tape_records_requests_and_answers() {
    let cc = compile(chart(vec![
        atomic("s0")
            .on(tr("e", "s1").cond("In('nope')"))
            .on(tr("e", "s1").cond("In('s0')")),
        atomic("s1"),
    ]));
    let mut null = NullDatamodel;
    let mut dm = TapeDatamodel::new(&mut null);
    let o = start(&cc, start_input(), &mut dm).unwrap();
    let start_tape = dm.take();
    assert!(matches!(start_tape[0].request, Request::InitSession(_)));
    assert!(matches!(start_tape[1].request, Request::SetEnvironment(_)));
    let o = step(
        &cc,
        o.session,
        Input::event(ev("e"), Environment { now_ms: 7, seed: 3 }),
        &mut dm,
    )
    .unwrap();
    let tape = dm.take();
    let reqs: Vec<(Request, Answer)> = tape.into_iter().map(|e| (e.request, e.answer)).collect();
    assert_eq!(
        reqs,
        vec![
            (
                Request::SetEnvironment(Environment { now_ms: 7, seed: 3 }),
                Answer::Unit
            ),
            (Request::SetEvent(ev("e")), Answer::Unit),
            (
                Request::EvalCond {
                    expr: "In('nope')".into()
                },
                Answer::Bool(false)
            ),
            (
                Request::EvalCond {
                    expr: "In('s0')".into()
                },
                Answer::Bool(true)
            ),
        ]
    );
    assert_eq!(o.trace.index, 1);
    assert_eq!(o.trace.configuration, ids(&["s1"]));
    assert_eq!(o.trace.microsteps[0].transitions[0].index, 1);
}
