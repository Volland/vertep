//! Further Appendix D and executable content behavior, beyond the spec scenarios.

use alloc::vec;
use alloc::vec::Vec;

use super::builder::*;
use crate::compile::CompiledChart;
use crate::datamodel::{Environment, Value};
use crate::ir::*;
use crate::session::{Effect, Input, start, step};

fn mock_chart(states: Vec<State>, data: Vec<(&str, &str)>) -> CompiledChart {
    let mut c = chart(states);
    c.datamodel = "mock".into();
    for (id, expr) in data {
        c.data.push(Data {
            id: id.into(),
            src: None,
            expr: Some(expr.into()),
            content: None,
            extensions: Vec::new(),
            span: Span::NONE,
        });
    }
    compile(c)
}

#[test]
fn foreach_binds_item_and_index_in_order() {
    let cc = mock_chart(
        vec![atomic("s").entry(vec![Exec::new(ExecKind::Foreach {
            array: "arr".into(),
            item: "it".into(),
            index: Some("i".into()),
            body: vec![log_expr("it"), log_expr("i")],
        })])],
        vec![],
    );
    let mut dm = MockDm::named("mock");
    dm.vars.insert(
        "arr".into(),
        Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
    );
    let o = start(&cc, start_input(), &mut dm).unwrap();
    let values: Vec<Value> = o
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Log { value: Some(v), .. } => Some(v.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        values,
        vec![
            Value::String("a".into()),
            Value::Number(0.0),
            Value::String("b".into()),
            Value::Number(1.0)
        ]
    );
}

#[test]
fn failing_if_condition_stops_the_block() {
    let cc = mock_chart(
        vec![
            atomic("s")
                .entry(vec![
                    if_(vec![
                        (Some("fail"), vec![log("then")]),
                        (None, vec![log("else")]),
                    ]),
                    log("after"),
                ])
                .on(tr("error.execution", "pass")),
            atomic("pass"),
        ],
        vec![],
    );
    let mut dm = MockDm::named("mock");
    let o = start(&cc, start_input(), &mut dm).unwrap();
    assert!(logs(&o.effects).is_empty());
    assert_eq!(active(&cc, &o), ids(&["pass"]));
}

#[test]
fn failing_guard_is_false_and_queues_error_execution() {
    let cc = mock_chart(
        vec![
            atomic("s")
                .on(tr("e", "fail").cond("fail"))
                .on(tr("e", "next")),
            atomic("next").on(tr("error.execution", "pass")),
            atomic("pass"),
            atomic("fail"),
        ],
        vec![],
    );
    let mut dm = MockDm::named("mock");
    let o = start(&cc, start_input(), &mut dm).unwrap();
    let o = step(
        &cc,
        o.session,
        Input::event(ev("e"), Environment::default()),
        &mut dm,
    )
    .unwrap();
    assert_eq!(events(&o), ids(&["e", "error.execution"]));
    assert_eq!(active(&cc, &o), ids(&["pass"]));
}

#[test]
fn send_idlocation_stores_the_generated_id() {
    let cc = mock_chart(
        vec![atomic("s").entry(vec![send(Send {
            idlocation: Some("sid_var".into()),
            target: Some("#_parent".into()),
            ..send_event("hello")
        })])],
        vec![("sid_var", "0")],
    );
    let mut dm = MockDm::named("mock");
    let o = start(&cc, start_input(), &mut dm).unwrap();
    let [Effect::Send(r)] = o.effects.as_slice() else {
        panic!("expected one send")
    };
    assert_eq!(r.target.as_deref(), Some("#_parent"));
    assert_eq!(r.event.sendid.as_deref(), Some(r.sendid.as_str()));
    assert_eq!(
        dm.vars.get("sid_var"),
        Some(&Value::String(r.sendid.clone()))
    );
}

#[test]
fn illegal_scxml_target_is_error_execution() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![send(Send {
                target: Some("baz".into()),
                ..send_event("e")
            })])
            .on(tr("error.execution", "pass")),
        atomic("pass"),
    ]));
    let o = run_null(&cc);
    assert!(o.effects.is_empty());
    assert_eq!(active(&cc, &o), ids(&["pass"]));
}

#[test]
fn history_in_parallel_restores_all_regions() {
    let cc = compile(chart(vec![
        parallel(
            "p",
            vec![
                history("h", HistoryType::Deep, "r1"),
                compound("r1", vec![atomic("x1").on(tr("go", "x2")), atomic("x2")]),
                compound("r2", vec![atomic("y1").on(tr("go", "y2")), atomic("y2")]),
            ],
        )
        .on(tr("out", "o")),
        atomic("o").on(tr("back", "h")),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "go");
    assert_eq!(active(&cc, &o), ids(&["p", "r1", "x2", "r2", "y2"]));
    let o = send_null(&cc, o.session, "out");
    let o = send_null(&cc, o.session, "back");
    assert_eq!(active(&cc, &o), ids(&["p", "r1", "x2", "r2", "y2"]));
}

#[test]
fn transition_into_parallel_region_enters_sibling_regions_by_default() {
    let cc = compile(chart(vec![
        atomic("s").on(tr("go", "y2")),
        parallel(
            "p",
            vec![
                compound("r1", vec![atomic("x1"), atomic("x2")]),
                compound("r2", vec![atomic("y1"), atomic("y2")]),
            ],
        ),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "go");
    assert_eq!(active(&cc, &o), ids(&["p", "r1", "x1", "r2", "y2"]));
}

#[test]
fn late_binding_data_initialized_before_onentry_once() {
    let mut s = atomic("s").on(tr("again", "t"));
    s.data.push(Data {
        id: "x".into(),
        src: None,
        expr: Some("1".into()),
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    });
    let s = s.entry(vec![log_expr("x"), assign("x", "2")]);
    let mut c = chart(vec![atomic("t").on(tr("go", "s")), s]);
    c.datamodel = "mock".into();
    c.binding = Binding::Late;
    let cc = compile(c);
    let mut dm = MockDm::named("mock");
    let mut session = start(&cc, start_input(), &mut dm).unwrap().session;
    let mut seen = Vec::new();
    for name in ["go", "again", "go"] {
        let o = step(
            &cc,
            session,
            Input::event(ev(name), Environment::default()),
            &mut dm,
        )
        .unwrap();
        for e in &o.effects {
            if let Effect::Log { value: Some(v), .. } = e {
                seen.push(v.clone());
            }
        }
        session = o.session;
    }
    // First entry sees the initial value; the second entry is not a first entry.
    assert_eq!(seen, vec![Value::Number(1.0), Value::Number(2.0)]);
}
