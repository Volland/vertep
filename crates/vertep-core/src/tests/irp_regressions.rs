//! Regression tests for interpreter and validator bugs found by the W3C SCXML IRP suite
//! (`vertep-conformance`). Each test names the IRP test that exposed the bug.

use alloc::string::ToString;
use alloc::vec;

use super::builder::*;
use crate::compile::CompiledChart;
use crate::ir::*;
use crate::validate::{ViolationKind, validate};

fn data(id: &str) -> Data {
    Data {
        id: id.to_string(),
        src: None,
        expr: None,
        content: None,
        extensions: vec![],
        span: Span::NONE,
    }
}

fn conflicts(c: &Chart) -> usize {
    match validate(c) {
        Ok(()) => 0,
        Err(v) => v
            .iter()
            .filter(|x| matches!(x.kind, ViolationKind::ConflictingAttributes { .. }))
            .count(),
    }
}

/// IRP 446, 552: a `<data src>` the reader resolved keeps the reference and its content.
#[test]
fn resolved_data_src_is_well_formed() {
    let mut c = chart(vec![atomic("s")]);
    c.data.push(Data {
        src: Some("file:test552.txt".into()),
        content: Some("[1, 2]".into()),
        ..data("Var1")
    });
    assert_eq!(conflicts(&c), 0);
    assert!(CompiledChart::new(c.clone()).is_ok());
    c.data[0].expr = Some("1".into());
    assert_eq!(conflicts(&c), 1, "src with expr still conflicts");
}

/// IRP 226, 239, 242, 276: an `<invoke src>` the reader resolved keeps the reference and the
/// document as an inline `<content>` body.
#[test]
fn resolved_invoke_src_is_well_formed() {
    let body = |expr: Option<&str>| Content {
        expr: expr.map(Into::into),
        body: if expr.is_some() {
            None
        } else {
            Some("<scxml/>".into())
        },
        span: Span::NONE,
    };
    let with = |inv: Invoke| chart(vec![atomic("s").invoke(inv)]);
    let resolved = Invoke {
        src: Some("file:sub.scxml".into()),
        content: Some(body(None)),
        ..Invoke::default()
    };
    assert_eq!(conflicts(&with(resolved)), 0);
    let srcexpr = Invoke {
        srcexpr: Some("x".into()),
        content: Some(body(None)),
        ..Invoke::default()
    };
    assert_eq!(conflicts(&with(srcexpr)), 1);
    let src_and_expr = Invoke {
        src: Some("file:sub.scxml".into()),
        content: Some(body(Some("x"))),
        ..Invoke::default()
    };
    assert_eq!(conflicts(&with(src_and_expr)), 1);
}

/// IRP 236: a child that finished before its state was exited still delivers the
/// `done.invoke.<id>` the host had already queued; the core does not discard it.
#[test]
fn done_invoke_queued_before_cancel_is_processed() {
    use crate::datamodel::{Environment, Event, EventType};
    use crate::null::NullDatamodel;
    use crate::session::{Effect, Input, step};
    let cc = compile(chart(vec![
        atomic("s").invoke(Invoke::default()).on(tr("child", "t")),
        atomic("t").on(tr("done.invoke", "done")),
        final_("done"),
    ]));
    let o = run_null(&cc);
    let Some(Effect::Invoke(req)) = o.effects.last() else {
        panic!("no invoke effect")
    };
    let id = req.invokeid.clone();
    let from_child = |name: &str| Event {
        invokeid: Some(id.clone()),
        ..Event::named(name, EventType::External)
    };
    let o = step(
        &cc,
        o.session,
        Input::event(from_child("child"), Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert_eq!(
        o.effects,
        vec![Effect::CancelInvoke {
            invokeid: id.clone()
        }]
    );
    let o = step(
        &cc,
        o.session,
        Input::event(
            from_child(&alloc::format!("done.invoke.{id}")),
            Environment::default(),
        ),
        &mut NullDatamodel,
    )
    .unwrap();
    assert!(o.trace.terminated, "done.invoke was processed");
}

fn history_without_default(id: &str) -> State {
    State::new(id, StateElement::History(HistoryType::Shallow))
}

fn has_violation(c: &Chart, f: impl Fn(&ViolationKind) -> bool) -> bool {
    validate(c)
        .err()
        .is_some_and(|v| v.iter().any(|x| f(&x.kind)))
}

/// A transition targeting a history state and a sibling of it would enter two children of the
/// same compound state; the history target takes its parent's position in the target-set rule.
#[test]
fn history_target_excludes_its_siblings() {
    let a = compound(
        "a",
        vec![
            atomic("a1"),
            atomic("a2"),
            history("h", HistoryType::Shallow, "a1"),
        ],
    );
    let illegal = |targets: &str| {
        let c = chart(vec![atomic("s").on(tr("e", targets)), a.clone()]);
        has_violation(&c, |k| matches!(k, ViolationKind::IllegalTargetSet { .. }))
    };
    assert!(illegal("h a2"), "history and its sibling");
    assert!(illegal("a2 h"), "order does not matter");
    assert!(!illegal("h"), "a history target alone is fine");
    // A history state and a target in another region of a parallel state are compatible.
    let p = parallel(
        "p",
        vec![
            compound(
                "r1",
                vec![atomic("x"), history("hr", HistoryType::Shallow, "x")],
            ),
            compound("r2", vec![atomic("y"), atomic("z")]),
        ],
    );
    let c = chart(vec![atomic("s").on(tr("e", "hr z")), p.clone()]);
    assert!(validate(&c).is_ok(), "{:?}", validate(&c));
    let c = chart(vec![atomic("s").on(tr("e", "hr x")), p]);
    assert!(has_violation(&c, |k| matches!(
        k,
        ViolationKind::IllegalTargetSet { .. }
    )));
}

/// `<state initial="h">` whose history child `h` has no default transition: the synthesized
/// default is the parent's `initial`, which is `h` again. Rejected instead of recursing forever,
/// also when the cycle runs through another history state's default.
#[test]
fn history_default_cycles_are_rejected() {
    let mut a = compound("a", vec![atomic("a1"), history_without_default("h")]);
    a.initial = vec!["h".to_string()];
    let c = chart(vec![a]);
    assert!(has_violation(&c, |k| matches!(
        k,
        ViolationKind::HistoryDefaultCycle { history } if history == "h"
    )));
    assert!(CompiledChart::new(c).is_err());

    let mut b = compound(
        "b",
        vec![
            atomic("b1"),
            history_without_default("h1"),
            history("h2", HistoryType::Shallow, "h1"),
        ],
    );
    b.initial = vec!["h2".to_string()];
    let c = chart(vec![b]);
    assert!(has_violation(&c, |k| matches!(
        k,
        ViolationKind::HistoryDefaultCycle { .. }
    )));

    // A history state without a default whose parent's initial is a real state is fine and
    // runs: entering it enters the parent's default.
    let ok = chart(vec![
        atomic("s").on(tr("", "h")),
        compound("c", vec![atomic("c1"), history_without_default("h")]),
    ]);
    let cc = compile(ok);
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["c", "c1"]));
}

/// IRP 496: with the reachable sessions listed, a send to `#_scxml_<id>` of no listed session
/// queues `error.communication` (with the send id) before a following `<raise>`; without the
/// list the send is left to the host.
#[test]
fn unknown_scxml_session_is_error_communication() {
    use crate::session::{Effect, StartInput, start};
    let target = |id: &str| Send {
        target: Some(alloc::format!("#_scxml_{id}")),
        ..send_event("ev")
    };
    let build = |id: &str| {
        compile(chart(vec![
            atomic("s0")
                .entry(vec![send(target(id)), raise("foo")])
                .on(tr("error.communication", "pass"))
                .on(tr("*", "fail")),
            final_("pass"),
            final_("fail"),
        ]))
    };
    let listed = |cc: &CompiledChart| {
        let input = StartInput {
            sessions: Some(vec!["sid".to_string()]),
            ..start_input()
        };
        start(cc, input, &mut crate::null::NullDatamodel).unwrap()
    };
    let cc = build("foo");
    let o = listed(&cc);
    assert_eq!(events(&o), ids(&["error.communication"]));
    assert!(o.trace.terminated);
    assert!(!o.effects.iter().any(|e| matches!(e, Effect::Send(_))));
    // A listed session (here the session itself) is dispatched normally.
    let cc = build("sid");
    let o = listed(&cc);
    assert!(o.effects.iter().any(|e| matches!(e, Effect::Send(_))));
    // Without the list the host decides.
    let cc = build("foo");
    let o = run_null(&cc);
    assert!(o.effects.iter().any(|e| matches!(e, Effect::Send(_))));
    assert_eq!(events(&o), ids(&["foo"]));
}

/// `error.execution` carries `{kind, message}` with the datamodel's error message, so the
/// cause of a failure is visible in `_event.data` (cel-datamodel, type-system `error.*`).
#[test]
fn execution_error_carries_the_datamodel_message() {
    use crate::datamodel::{Event, Value};
    use crate::null::NullDatamodel;
    use crate::session::start;
    use crate::tape::{Request, TapeDatamodel};

    let mut s = atomic("s");
    s.onentry.push(vec![log_expr("1 + 1")]);
    s.transitions.push(tr("error.execution", "pass"));
    let cc = compile(chart(vec![s, final_("pass")]));
    let mut null = NullDatamodel;
    let mut tape = TapeDatamodel::new(&mut null);
    let o = start(&cc, start_input(), &mut tape).expect("start");
    assert_eq!(events(&o), ids(&["error.execution"]));
    let err: &Event = tape
        .entries()
        .iter()
        .find_map(|e| match &e.request {
            Request::SetEvent(ev) if ev.name == "error.execution" => Some(ev),
            _ => None,
        })
        .expect("error.execution was delivered");
    let Value::Object(fields) = &err.data else {
        panic!("error data is not an object: {:?}", err.data)
    };
    assert_eq!(
        fields[0],
        ("kind".into(), Value::String("execution".into()))
    );
    let Value::String(message) = &fields[1].1 else {
        panic!("message is not a string")
    };
    assert_eq!(fields[1].0, "message");
    assert!(!message.is_empty() && message != crate::session::EXECUTION_ERROR_MESSAGE);
}

/// Found by the Rust/Lean differential tests: an eventless transition whose guard fails queues
/// `error.execution`, which enables nothing, after which the guard fails again. Every consumed
/// event counts against the microstep budget, so the macrostep ends instead of looping forever.
#[test]
fn failing_eventless_guard_loop_hits_the_budget() {
    use crate::session::{StartError, start};
    let cc = compile(chart(vec![
        atomic("s0").on(tr("", "s1").cond("not_an_in_predicate")),
        atomic("s1"),
    ]));
    let mut input = start_input();
    input.options.microstep_budget = 50;
    match start(&cc, input, &mut crate::null::NullDatamodel) {
        Err(StartError::BudgetExceeded { budget, partial }) => {
            assert_eq!(budget, 50);
            assert_eq!(partial.trace.microsteps.len(), 50);
            assert!(
                partial
                    .trace
                    .microsteps
                    .iter()
                    .all(|m| m.event.as_deref() == Some("error.execution"))
            );
        }
        other => panic!("expected the budget to run out, got {other:?}"),
    }
}
