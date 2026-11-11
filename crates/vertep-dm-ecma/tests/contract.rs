//! The datamodel contract seen from the interpreter: every datamodel is driven through the same
//! `Datamodel` trait, and failures come back as evaluation errors, never panics.

use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Datamodel, Environment, SessionInfo, Value};
use vertep_core::ir::{Chart, Exec, ExecKind, State, StateElement, Transition};
use vertep_core::null::NullDatamodel;
use vertep_core::session::{Options, Outcome, StartInput, start};
use vertep_core::tape::{Request, TapeDatamodel};
use vertep_dm_ecma::EcmaDatamodel;

fn transition(event: &str, cond: Option<&str>, target: &str) -> Transition {
    Transition {
        events: if event.is_empty() {
            vec![]
        } else {
            vec![event.into()]
        },
        cond: cond.map(Into::into),
        targets: vec![target.into()],
        ..Transition::default()
    }
}

fn log(expr: &str) -> Exec {
    Exec::new(ExecKind::Log {
        label: Some("probe".into()),
        expr: Some(expr.into()),
    })
}

/// A parallel state whose first region moves on only when `In('b')` holds, the one condition
/// both datamodels understand.
fn in_chart(datamodel: &str) -> Chart {
    let mut a = State::new("a", StateElement::State);
    a.transitions.push(transition("", Some("In('b')"), "a2"));
    let mut r1 = State::new("r1", StateElement::State);
    r1.children = vec![a, State::new("a2", StateElement::State)];
    let mut r2 = State::new("r2", StateElement::State);
    r2.children = vec![State::new("b", StateElement::State)];
    let mut p = State::new("p", StateElement::Parallel);
    p.children = vec![r1, r2];
    Chart {
        datamodel: datamodel.into(),
        datamodel_explicit: true,
        states: vec![p],
        ..Chart::default()
    }
}

fn start_input() -> StartInput {
    StartInput {
        info: SessionInfo {
            sessionid: "sid".into(),
            name: None,
            ioprocessors: Value::Undefined,
        },
        env: Environment::default(),
        options: Options::default(),
        data: Vec::new(),
        sessions: None,
    }
}

fn run(chart: Chart, dm: &mut dyn Datamodel) -> (Outcome, Vec<Request>) {
    let cc = CompiledChart::new(chart).expect("well-formed");
    let mut tape = TapeDatamodel::new(dm);
    let o = start(&cc, start_input(), &mut tape).expect("start");
    let requests = tape.entries().iter().map(|e| e.request.clone()).collect();
    (o, requests)
}

fn kind(r: &Request) -> core::mem::Discriminant<Request> {
    core::mem::discriminant(r)
}

// @tg: verifies:: [[openspec:datamodel#Datamodel Contract#Same contract for every datamodel]] {level: integration}
#[test]
fn same_contract_for_every_datamodel() {
    let mut null = NullDatamodel;
    let mut ecma = EcmaDatamodel::new();
    let (on, rn) = run(in_chart("null"), &mut null);
    let (oe, re) = run(in_chart("ecmascript"), &mut ecma);

    // Same configuration from both datamodels.
    assert_eq!(on.session.configuration, oe.session.configuration);
    // The interpreter issued the same sequence of contract operations to both.
    assert_eq!(
        rn.iter().map(kind).collect::<Vec<_>>(),
        re.iter().map(kind).collect::<Vec<_>>()
    );
    assert!(
        rn.iter()
            .any(|r| matches!(r, Request::EvalCond { expr } if expr == "In('b')"))
    );
}

// @tg: verifies:: [[openspec:datamodel#Datamodel Contract#Evaluation error instead of panic]] {level: integration}
#[test]
fn evaluation_error_instead_of_panic() {
    let mut s = State::new("s", StateElement::State);
    s.onentry
        .push(vec![log("noSuchVariable.field"), log("'not reached'")]);
    s.transitions
        .push(transition("error.execution", None, "pass"));
    let chart = Chart {
        datamodel: "ecmascript".into(),
        datamodel_explicit: true,
        states: vec![s, State::new("pass", StateElement::Final)],
        ..Chart::default()
    };
    let mut ecma = EcmaDatamodel::new();
    // Called directly, the datamodel reports an error value.
    struct NoStates;
    impl vertep_core::datamodel::StateQuery for NoStates {
        fn in_state(&self, _: &str) -> bool {
            false
        }
    }
    assert!(ecma.eval_value("noSuchVariable.field", &NoStates).is_err());

    // Through the interpreter: the block stops, error.execution is raised, and the session goes on.
    let (o, _) = run(chart, &mut EcmaDatamodel::new());
    let logs: Vec<_> = o
        .effects
        .iter()
        .filter(|e| matches!(e, vertep_core::session::Effect::Log { .. }))
        .collect();
    assert!(
        logs.is_empty(),
        "the failing log and the rest of its block do not run"
    );
    let ended = o.effects.iter().any(|e| {
        matches!(
            e,
            vertep_core::session::Effect::SessionDone {
                cancelled: false,
                ..
            }
        )
    });
    assert!(ended, "the interpreter continued to the final state");
}
