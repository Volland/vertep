//! One test per scxml-interpretation scenario.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::builder::*;
use crate::datamodel::{Environment, Event, EventType, Value};
use crate::ir::*;
use crate::kernel::descriptor_matches;
use crate::null::NullDatamodel;
use crate::session::{Effect, Input, Options, StartError, StepError, parse_delay, start, step};
use crate::tape::{Request, TapeDatamodel};
use crate::trace::TransitionRef;

fn invoke_effects(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Invoke(r) => Some(r.invokeid.clone()),
            _ => None,
        })
        .collect()
}

// ----- Pure Step Function -------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Pure Step Function#Same input, same output]] {level: unit}
#[test]
fn same_input_same_output() {
    let cc = compile(chart(vec![
        atomic("s0").on(tr("go", "s1").body(vec![
            send(Send {
                delay: Some("1s".into()),
                ..send_event("later")
            }),
            raise("inner"),
        ])),
        atomic("s1").on(tr("inner", "s2").body(vec![log("inner seen")])),
        atomic("s2"),
    ]));
    let session = run_null(&cc).session;
    let a = step(
        &cc,
        session.clone(),
        Input::event(ev("go"), Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    let b = step(
        &cc,
        session,
        Input::event(ev("go"), Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert_eq!(a, b);
    assert_eq!(active(&cc, &a), ids(&["s2"]));
    assert_eq!(a.effects.len(), 2);
}

// @tg: verifies:: [[openspec:scxml-interpretation#Pure Step Function#No side effects outside the result]] {level: unit}
#[test]
fn no_side_effects_outside_the_result() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![send(send_event("out")), log("hello")])
            .invoke(Invoke::default()),
    ]));
    let o = run_null(&cc);
    assert_eq!(o.effects.len(), 3);
    assert!(matches!(&o.effects[0], Effect::Send(r) if r.event.name == "out"));
    assert!(matches!(&o.effects[1], Effect::Log { label: Some(l), .. } if l == "hello"));
    assert!(matches!(&o.effects[2], Effect::Invoke(r) if r.state == "s"));
    // The trace carries the same effects, and nothing else happened.
    assert_eq!(o.trace.effects, o.effects);
}

// ----- Session Initialization ---------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Session Initialization#Initial configuration entered]] {level: unit}
#[test]
fn initial_configuration_entered() {
    let cc = compile(chart(vec![
        compound("a", vec![atomic("a1").logged(), atomic("a2").logged()]).logged(),
    ]));
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["a", "a1"]));
    assert_eq!(logs(&o.effects), ids(&["enter a", "enter a1"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Session Initialization#Eventless transitions taken at start]] {level: unit}
#[test]
fn eventless_transitions_taken_at_start() {
    let cc = compile(chart(vec![
        atomic("s0").on(tr("", "s1").cond("In('s0')")),
        atomic("s1"),
    ]));
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["s1"]));
    assert_eq!(o.trace.microsteps.len(), 1);
    assert_eq!(o.trace.microsteps[0].event, None);
}

// ----- Macrostep Execution ------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Macrostep Execution#Internal events before the next external event]] {level: unit}
#[test]
fn internal_events_before_the_next_external_event() {
    let cc = compile(chart(vec![
        atomic("s0").on(tr("e1", "s1").body(vec![raise("i1"), raise("i2")])),
        atomic("s1")
            .on(tr("i1", "s2").body(vec![log("i1")]))
            .on(tr("e2", "fail")),
        atomic("s2")
            .on(tr("i2", "s3").body(vec![log("i2")]))
            .on(tr("e2", "fail")),
        atomic("s3").on(tr("e2", "s4")),
        atomic("s4"),
        atomic("fail"),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e1");
    assert_eq!(events(&o), ids(&["e1", "i1", "i2"]));
    assert_eq!(logs(&o.effects), ids(&["i1", "i2"]));
    assert!(o.session.internal_queue.is_empty());
    let o = send_null(&cc, o.session, "e2");
    assert_eq!(active(&cc, &o), ids(&["s4"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Macrostep Execution#Eventless transitions take priority over internal events]] {level: unit}
#[test]
fn eventless_transitions_take_priority_over_internal_events() {
    let cc = compile(chart(vec![
        atomic("s0")
            .entry(vec![raise("x")])
            .on(tr("x", "fail"))
            .on(tr("", "s1")),
        atomic("s1").on(tr("x", "pass")),
        atomic("pass"),
        atomic("fail"),
    ]));
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["pass"]));
    let order: Vec<Option<String>> = o.trace.microsteps.iter().map(|m| m.event.clone()).collect();
    assert_eq!(order, vec![None, Some("x".into())]);
}

// @tg: verifies:: [[openspec:scxml-interpretation#Macrostep Execution#Unbounded eventless loop is cut by a budget]] {level: unit}
#[test]
fn unbounded_eventless_loop_is_cut_by_a_budget() {
    let cc = compile(chart(vec![
        atomic("a").on(tr("", "b")),
        atomic("b").on(tr("", "a")),
    ]));
    let mut input = start_input();
    input.options = Options {
        microstep_budget: 50,
        ..Options::default()
    };
    match start(&cc, input, &mut NullDatamodel) {
        Err(StartError::BudgetExceeded { budget, partial }) => {
            assert_eq!(budget, 50);
            assert_eq!(partial.trace.microsteps.len(), 50);
        }
        other => panic!("expected a budget error, got {other:?}"),
    }
    // The budget also applies to steps.
    let cc = compile(chart(vec![
        atomic("s").on(tr("go", "a")),
        atomic("a").on(tr("", "b")),
        atomic("b").on(tr("", "a")),
    ]));
    let o = run_null(&cc);
    let r = step(
        &cc,
        o.session,
        Input::event(ev("go"), Environment::default()),
        &mut NullDatamodel,
    );
    assert!(matches!(
        r,
        Err(StepError::BudgetExceeded {
            budget: crate::session::DEFAULT_MICROSTEP_BUDGET,
            ..
        })
    ));
}

// ----- Event Descriptor Matching ------------------------------------------------------------

fn error_catcher(descriptor: &str) -> crate::compile::CompiledChart {
    compile(chart(vec![
        atomic("s")
            .entry(vec![log_expr("not an In() expression")])
            .on(tr(descriptor, "caught"))
            .on(tr("*", "other")),
        atomic("caught"),
        atomic("other"),
    ]))
}

// @tg: verifies:: [[openspec:scxml-interpretation#Event Descriptor Matching#Prefix match]] {level: unit}
#[test]
fn prefix_match() {
    assert!(descriptor_matches("error", "error.execution"));
    assert!(descriptor_matches("error.execution", "error.execution"));
    assert!(descriptor_matches("error.*", "error.execution"));
    let cc = error_catcher("error");
    assert_eq!(active(&cc, &run_null(&cc)), ids(&["caught"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Event Descriptor Matching#Partial token does not match]] {level: unit}
#[test]
fn partial_token_does_not_match() {
    assert!(!descriptor_matches("err", "error.execution"));
    assert!(!descriptor_matches("error.exec", "error.execution"));
    assert!(!descriptor_matches(
        "error.execution.more",
        "error.execution"
    ));
    let cc = error_catcher("err");
    assert_eq!(active(&cc, &run_null(&cc)), ids(&["other"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Event Descriptor Matching#Wildcard]] {level: unit}
#[test]
fn wildcard() {
    for name in ["a", "error.execution", "done.state.x", "x.y.z"] {
        assert!(descriptor_matches("*", name));
    }
    let cc = compile(chart(vec![atomic("s").on(tr("*", "t")), atomic("t")]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "anything.at.all");
    assert_eq!(active(&cc, &o), ids(&["t"]));
}

// ----- Optimal Enabled Transition Set -------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Optimal Enabled Transition Set#Descendant wins over ancestor]] {level: unit}
#[test]
fn descendant_wins_over_ancestor() {
    let cc = compile(chart(vec![
        compound("a", vec![atomic("a1").on(tr("e", "x"))]).on(tr("e", "y")),
        atomic("x"),
        atomic("y"),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(active(&cc, &o), ids(&["x"]));
    assert_eq!(
        o.trace.microsteps[0].transitions,
        vec![TransitionRef {
            source: "a1".into(),
            index: 0
        }]
    );
}

fn two_regions(t1: &str, t2: &str) -> Vec<State> {
    vec![
        parallel(
            "p",
            vec![
                compound("r1", vec![atomic("r1a").on(tr("e", t1)), atomic("r1b")]),
                compound("r2", vec![atomic("r2a").on(tr("e", t2)), atomic("r2b")]),
            ],
        ),
        atomic("out1"),
        atomic("out2"),
    ]
}

// @tg: verifies:: [[openspec:scxml-interpretation#Optimal Enabled Transition Set#Parallel regions take transitions together]] {level: unit}
#[test]
fn parallel_regions_take_transitions_together() {
    let cc = compile(chart(two_regions("r1b", "r2b")));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(active(&cc, &o), ids(&["p", "r1", "r1b", "r2", "r2b"]));
    assert_eq!(o.trace.microsteps.len(), 1);
    assert_eq!(o.trace.microsteps[0].transitions.len(), 2);
}

// @tg: verifies:: [[openspec:scxml-interpretation#Optimal Enabled Transition Set#Conflict resolved by document order]] {level: unit}
#[test]
fn conflict_resolved_by_document_order() {
    let cc = compile(chart(two_regions("out1", "out2")));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(active(&cc, &o), ids(&["out1"]));
    assert_eq!(
        o.trace.microsteps[0].transitions,
        vec![TransitionRef {
            source: "r1a".into(),
            index: 0
        }]
    );
}

// @tg: verifies:: [[openspec:scxml-interpretation#Optimal Enabled Transition Set#Condition false skips the transition]] {level: unit}
#[test]
fn condition_false_skips_the_transition() {
    let cc = compile(chart(vec![
        atomic("s0")
            .on(tr("e", "fail").cond("In('pass')"))
            .on(tr("e", "pass")),
        atomic("pass"),
        atomic("fail"),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(active(&cc, &o), ids(&["pass"]));
}

// ----- Exit And Entry Order -----------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Exit And Entry Order#Exit deepest first]] {level: unit}
#[test]
fn exit_deepest_first() {
    let cc = compile(chart(vec![
        compound("a", vec![atomic("a1").logged().on(tr("e", "b"))]).logged(),
        atomic("b"),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(logs(&o.effects), ids(&["exit a1", "exit a"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Exit And Entry Order#Transition content between exit and entry]] {level: unit}
#[test]
fn transition_content_between_exit_and_entry() {
    let cc = compile(chart(vec![
        atomic("a1")
            .logged()
            .on(tr("e", "b1").body(vec![log("transition")])),
        atomic("b1").logged(),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(
        logs(&o.effects),
        ids(&["exit a1", "transition", "enter b1"])
    );
}

// @tg: verifies:: [[openspec:scxml-interpretation#Exit And Entry Order#Parallel regions entered in document order]] {level: unit}
#[test]
fn parallel_regions_entered_in_document_order() {
    let cc = compile(chart(vec![
        parallel(
            "p",
            vec![
                compound("r1", vec![atomic("r1a").logged()]).logged(),
                compound("r2", vec![atomic("r2a").logged()]).logged(),
            ],
        )
        .logged(),
    ]));
    let o = run_null(&cc);
    assert_eq!(
        logs(&o.effects),
        ids(&["enter p", "enter r1", "enter r1a", "enter r2", "enter r2a"])
    );
}

// ----- Internal Transitions -----------------------------------------------------------------

fn internal_chart(internal: bool) -> crate::compile::CompiledChart {
    let t = tr("go", "a2");
    let t = if internal { t.internal() } else { t };
    compile(chart(vec![
        compound("a", vec![atomic("a1").logged(), atomic("a2").logged()])
            .logged()
            .on(t),
    ]))
}

// @tg: verifies:: [[openspec:scxml-interpretation#Internal Transitions#Internal transition keeps the source]] {level: unit}
#[test]
fn internal_transition_keeps_the_source() {
    let cc = internal_chart(true);
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "go");
    assert_eq!(logs(&o.effects), ids(&["exit a1", "enter a2"]));
    assert_eq!(active(&cc, &o), ids(&["a", "a2"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Internal Transitions#External transition re-enters the source]] {level: unit}
#[test]
fn external_transition_re_enters_the_source() {
    let cc = internal_chart(false);
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "go");
    assert_eq!(
        logs(&o.effects),
        ids(&["exit a1", "exit a", "enter a", "enter a2"])
    );
    assert_eq!(active(&cc, &o), ids(&["a", "a2"]));
}

// ----- History Restoration ------------------------------------------------------------------

fn history_chart(ty: HistoryType) -> crate::compile::CompiledChart {
    compile(chart(vec![
        compound(
            "a",
            vec![
                history("h", ty, "a2").on_default_body(vec![log("history default")]),
                atomic("a1").on(tr("to2", "a2")),
                compound("a2", vec![atomic("a2x").on(tr("y", "a2y")), atomic("a2y")]),
            ],
        )
        .on(tr("out", "b")),
        atomic("b").on(tr("back", "h")),
    ]))
}

trait HistoryBody {
    fn on_default_body(self, b: Block) -> Self;
}

impl HistoryBody for State {
    fn on_default_body(mut self, b: Block) -> Self {
        self.transitions[0].body = b;
        self
    }
}

fn leave_from_a2y(cc: &crate::compile::CompiledChart) -> crate::session::Session {
    let o = run_null(cc);
    let o = send_null(cc, o.session, "to2");
    let o = send_null(cc, o.session, "y");
    assert_eq!(active(cc, &o), ids(&["a", "a2", "a2y"]));
    let o = send_null(cc, o.session, "out");
    assert_eq!(active(cc, &o), ids(&["b"]));
    o.session
}

// @tg: verifies:: [[openspec:scxml-interpretation#History Restoration#Shallow history restores the child]] {level: unit}
#[test]
fn shallow_history_restores_the_child() {
    let cc = history_chart(HistoryType::Shallow);
    let s = leave_from_a2y(&cc);
    let o = send_null(&cc, s, "back");
    assert_eq!(active(&cc, &o), ids(&["a", "a2", "a2x"]));
    assert!(logs(&o.effects).is_empty());
}

// @tg: verifies:: [[openspec:scxml-interpretation#History Restoration#Deep history restores the leaf]] {level: unit}
#[test]
fn deep_history_restores_the_leaf() {
    let cc = history_chart(HistoryType::Deep);
    let s = leave_from_a2y(&cc);
    let o = send_null(&cc, s, "back");
    assert_eq!(active(&cc, &o), ids(&["a", "a2", "a2y"]));
    assert!(logs(&o.effects).is_empty());
}

// @tg: verifies:: [[openspec:scxml-interpretation#History Restoration#Default transition when no history recorded]] {level: unit}
#[test]
fn default_transition_when_no_history_recorded() {
    let mut c = chart(vec![]);
    let hc = history_chart(HistoryType::Deep);
    c.states = hc.chart().states.clone();
    c.initial = vec!["b".into()];
    let cc = compile(c);
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["b"]));
    let o = send_null(&cc, o.session, "back");
    assert_eq!(active(&cc, &o), ids(&["a", "a2", "a2x"]));
    assert_eq!(logs(&o.effects), ids(&["history default"]));
}

// ----- Final States And Done Events ---------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Final States And Done Events#Compound done event]] {level: unit}
#[test]
fn compound_done_event() {
    let cc = compile(chart(vec![
        compound(
            "a",
            vec![
                atomic("a1").on(tr("e", "f")),
                final_("f").donedata("result"),
            ],
        )
        .on(tr("done.state.a", "pass")),
        atomic("pass"),
    ]));
    let mut null = NullDatamodel;
    let mut dm = TapeDatamodel::new(&mut null);
    let o = start(&cc, start_input(), &mut dm).unwrap();
    let o = step(
        &cc,
        o.session,
        Input::event(ev("e"), Environment::default()),
        &mut dm,
    )
    .unwrap();
    assert_eq!(events(&o), ids(&["e", "done.state.a"]));
    assert_eq!(active(&cc, &o), ids(&["pass"]));
    let done = dm
        .entries()
        .iter()
        .find_map(|e| match &e.request {
            Request::SetEvent(ev) if ev.name == "done.state.a" => Some(ev.clone()),
            _ => None,
        })
        .expect("done.state.a was bound to _event");
    assert_eq!(done.data, Value::String("result".into()));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Final States And Done Events#Parallel done event]] {level: unit}
#[test]
fn parallel_done_event() {
    let cc = compile(chart(vec![
        parallel(
            "p",
            vec![
                compound("r1", vec![atomic("r1a").on(tr("e1", "r1f")), final_("r1f")]),
                compound("r2", vec![atomic("r2a").on(tr("e2", "r2f")), final_("r2f")]),
            ],
        )
        .on(tr("done.state.p", "pass")),
        atomic("pass"),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e1");
    assert_eq!(events(&o), ids(&["e1", "done.state.r1"]));
    assert!(o.session.is_active(&cc, "p"));
    let o = send_null(&cc, o.session, "e2");
    assert_eq!(events(&o), ids(&["e2", "done.state.r2", "done.state.p"]));
    assert_eq!(active(&cc, &o), ids(&["pass"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Final States And Done Events#Top-level final ends the session]] {level: unit}
#[test]
fn top_level_final_ends_the_session() {
    let cc = compile(chart(vec![
        atomic("s").on(tr("e", "end")),
        final_("end").donedata("bye").logged(),
    ]));
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert!(o.session.is_terminated());
    assert!(o.trace.terminated);
    assert_eq!(logs(&o.effects), ids(&["enter end", "exit end"]));
    assert_eq!(
        o.effects.last(),
        Some(&Effect::SessionDone {
            data: Value::String("bye".into()),
            cancelled: false
        })
    );
}

// ----- Internal And External Queues ---------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Internal And External Queues#Raise is internal]] {level: unit}
#[test]
fn raise_is_internal() {
    let cc = compile(chart(vec![
        atomic("s").entry(vec![raise("x")]).on(tr("x", "pass")),
        atomic("pass"),
    ]));
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["pass"]));
    assert!(o.effects.is_empty());
    assert_eq!(events(&o), ids(&["x"]));
}

// @tg: verifies:: [[openspec:scxml-interpretation#Internal And External Queues#Send to the internal target]] {level: unit}
#[test]
fn send_to_the_internal_target() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![send(Send {
                target: Some("#_internal".into()),
                ..send_event("x")
            })])
            .on(tr("x", "pass")),
        atomic("pass"),
    ]));
    let mut null = NullDatamodel;
    let mut dm = TapeDatamodel::new(&mut null);
    let o = start(&cc, start_input(), &mut dm).unwrap();
    assert_eq!(active(&cc, &o), ids(&["pass"]));
    assert!(o.effects.is_empty());
    let bound = dm
        .entries()
        .iter()
        .find_map(|e| match &e.request {
            Request::SetEvent(ev) => Some(ev.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(bound.name, "x");
    assert_eq!(bound.type_, EventType::Internal);
}

// ----- Execution Errors ---------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Execution Errors#Failed expression stops the block]] {level: unit}
#[test]
fn failed_expression_stops_the_block() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![log("first"), log_expr("1 + 1"), log("third")])
            .entry(vec![log("next block")])
            .on(tr("error.execution", "pass")),
        atomic("pass"),
    ]));
    let o = run_null(&cc);
    assert_eq!(logs(&o.effects), ids(&["first", "next block"]));
    assert_eq!(events(&o), ids(&["error.execution"]));
    assert_eq!(active(&cc, &o), ids(&["pass"]));
}

/// The spec scenario says `error.communication`; W3C SCXML 6.2.4 and IRP test 199 require
/// `error.execution` for a type no Event I/O Processor supports, and the Recommendation is
/// normative, so this test checks the W3C behavior (see the `-contradicts` edge on `exec_send`).
// @tg: verifies:: [[openspec:scxml-interpretation#Execution Errors#Invalid send type]] {level: unit}
#[test]
fn invalid_send_type() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![
                send(Send {
                    type_: Some("27".into()),
                    ..send_event("event1")
                }),
                log("not reached"),
            ])
            .on(tr("error", "caught")),
        atomic("caught").on(tr("again", "s")),
    ]));
    let mut null = NullDatamodel;
    let mut dm = TapeDatamodel::new(&mut null);
    let o = start(&cc, start_input(), &mut dm).unwrap();
    assert_eq!(active(&cc, &o), ids(&["caught"]));
    assert!(o.effects.is_empty());
    assert_eq!(events(&o), ids(&["error.execution"]));
    // The error event carries the send identifier of the failed send.
    let err = dm
        .entries()
        .iter()
        .find_map(|e| match &e.request {
            Request::SetEvent(ev) => Some(ev.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(err.type_, EventType::Platform);
    assert_eq!(err.sendid.as_deref(), Some("sid.send.1"));
    // The session continues: it still processes events.
    let o = step(
        &cc,
        o.session,
        Input::event(ev("again"), Environment::default()),
        &mut dm,
    )
    .unwrap();
    assert!(!o.session.is_terminated());
    assert_eq!(events(&o), ids(&["again", "error.execution"]));
}

// ----- Send And Cancel Effects --------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Send And Cancel Effects#Delayed send becomes an effect]] {level: unit}
#[test]
fn delayed_send_becomes_an_effect() {
    let cc = compile(chart(vec![atomic("s").entry(vec![send(Send {
        delay: Some("5s".into()),
        ..send_event("tick")
    })])]));
    let o = run_null(&cc);
    let [Effect::Send(r)] = o.effects.as_slice() else {
        panic!("expected one send effect, got {:?}", o.effects)
    };
    assert_eq!(r.delay_ms, 5000);
    assert_eq!(r.event.name, "tick");
    assert_eq!(r.sendid, "sid.send.1");
    assert_eq!(r.target, None);
    assert_eq!(r.event.origin.as_deref(), Some("#_scxml_sid"));
    assert_eq!(parse_delay("1.5s"), Some(1500));
    assert_eq!(parse_delay("250ms"), Some(250));
    assert_eq!(parse_delay("soon"), None);
}

// @tg: verifies:: [[openspec:scxml-interpretation#Send And Cancel Effects#Deterministic send identifiers]] {level: unit}
#[test]
fn deterministic_send_identifiers() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![send(send_event("a")), send(send_event("b"))])
            .on(tr("go", "s")),
    ]));
    let run = || {
        let o = run_null(&cc);
        let mut all = o.effects.clone();
        all.extend(send_null(&cc, o.session, "go").effects);
        all.iter()
            .filter_map(|e| match e {
                Effect::Send(r) => Some(r.sendid.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let first = run();
    assert_eq!(
        first,
        ids(&["sid.send.1", "sid.send.2", "sid.send.3", "sid.send.4"])
    );
    assert_eq!(first, run());
}

// @tg: verifies:: [[openspec:scxml-interpretation#Send And Cancel Effects#Cancel by send identifier]] {level: unit}
#[test]
fn cancel_by_send_identifier() {
    let cc = compile(chart(vec![atomic("s").entry(vec![Exec::new(
        ExecKind::Cancel {
            sendid: Some("s1".into()),
            sendidexpr: None,
        },
    )])]));
    let o = run_null(&cc);
    assert_eq!(
        o.effects,
        vec![Effect::Cancel {
            sendid: "s1".into()
        }]
    );
}

// ----- Invoke Lifecycle ---------------------------------------------------------------------

fn finalize_invoke() -> Invoke {
    Invoke {
        finalize: vec![log("finalize")],
        autoforward: true,
        ..Invoke::default()
    }
}

// @tg: verifies:: [[openspec:scxml-interpretation#Invoke Lifecycle#Invoke started after the macrostep]] {level: unit}
#[test]
fn invoke_started_after_the_macrostep() {
    let cc = compile(chart(vec![
        atomic("s")
            .entry(vec![log("enter s"), raise("r")])
            .on(tr("r", "").body(vec![log("handled r")]))
            .invoke(Invoke::default()),
    ]));
    let o = run_null(&cc);
    assert_eq!(logs(&o.effects), ids(&["enter s", "handled r"]));
    // The invoke comes after everything else in the macrostep, including the internal event.
    assert!(matches!(o.effects.last(), Some(Effect::Invoke(r)) if r.state == "s"));
    assert_eq!(o.session.invocations.len(), 1);
}

// @tg: verifies:: [[openspec:scxml-interpretation#Invoke Lifecycle#Invoke not started when the state is left in the same macrostep]] {level: unit}
#[test]
fn invoke_not_started_when_the_state_is_left_in_the_same_macrostep() {
    let cc = compile(chart(vec![
        atomic("s").invoke(Invoke::default()).on(tr("", "t")),
        atomic("t"),
    ]));
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["t"]));
    assert!(invoke_effects(&o.effects).is_empty());
    assert!(
        o.effects
            .iter()
            .all(|e| !matches!(e, Effect::CancelInvoke { .. }))
    );
}

// @tg: verifies:: [[openspec:scxml-interpretation#Invoke Lifecycle#Finalize runs before the event]] {level: unit}
#[test]
fn finalize_runs_before_the_event() {
    let cc = compile(chart(vec![
        atomic("s")
            .invoke(finalize_invoke())
            .on(tr("child.ev", "").body(vec![log("transition")])),
    ]));
    let o = run_null(&cc);
    let id = invoke_effects(&o.effects).pop().unwrap();
    let event = Event {
        invokeid: Some(id.clone()),
        ..Event::named("child.ev", EventType::External)
    };
    let o = step(
        &cc,
        o.session,
        Input::event(event, Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert_eq!(logs(&o.effects), ids(&["finalize", "transition"]));
    // Autoforward forwards external events to the invocation.
    assert!(
        o.effects
            .iter()
            .any(|e| matches!(e, Effect::Forward { invokeid, .. } if *invokeid == id))
    );
}

// @tg: verifies:: [[openspec:scxml-interpretation#Invoke Lifecycle#Exiting the state cancels the invocation]] {level: unit}
#[test]
fn exiting_the_state_cancels_the_invocation() {
    let cc = compile(chart(vec![
        atomic("s").invoke(Invoke::default()).on(tr("e", "t")),
        atomic("t"),
    ]));
    let o = run_null(&cc);
    let id = invoke_effects(&o.effects).pop().unwrap();
    let o = send_null(&cc, o.session, "e");
    assert_eq!(
        o.effects,
        vec![Effect::CancelInvoke {
            invokeid: id.clone()
        }]
    );
    assert!(o.session.invocations.is_empty());
    // Events from the cancelled invocation are ignored afterwards.
    let late = Event {
        invokeid: Some(id),
        ..Event::named("e", EventType::External)
    };
    let o = step(
        &cc,
        o.session,
        Input::event(late, Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert!(o.effects.is_empty());
    assert!(o.trace.microsteps[0].transitions.is_empty());
}

#[test]
fn done_invoke_completes_the_invocation() {
    let cc = compile(chart(vec![
        atomic("s")
            .invoke(Invoke::default())
            .on(tr("done.invoke", "t")),
        atomic("t"),
    ]));
    let o = run_null(&cc);
    let id = invoke_effects(&o.effects).pop().unwrap();
    let done = Event {
        invokeid: Some(id.clone()),
        ..Event::named(alloc::format!("done.invoke.{id}"), EventType::External)
    };
    let o = step(
        &cc,
        o.session,
        Input::event(done, Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert_eq!(active(&cc, &o), ids(&["t"]));
    // A completed invocation is not cancelled on exit.
    assert!(o.effects.is_empty());
}

// ----- Session Termination ------------------------------------------------------------------

// @tg: verifies:: [[openspec:scxml-interpretation#Session Termination#Exit content runs on termination]] {level: unit}
#[test]
fn exit_content_runs_on_termination() {
    let cc = compile(chart(vec![
        compound("a", vec![atomic("a1").logged()])
            .logged()
            .invoke(Invoke::default()),
    ]));
    let o = run_null(&cc);
    let id = invoke_effects(&o.effects).pop().unwrap();
    let o = step(
        &cc,
        o.session,
        Input::cancel(Environment::default()),
        &mut NullDatamodel,
    )
    .unwrap();
    assert_eq!(logs(&o.effects), ids(&["exit a1", "exit a"]));
    assert_eq!(
        o.effects[2..],
        [
            Effect::CancelInvoke { invokeid: id },
            Effect::SessionDone {
                data: Value::Undefined,
                cancelled: true
            }
        ]
    );
    assert!(o.session.is_terminated());
    assert!(o.session.configuration.is_empty());
}

// @tg: verifies:: [[openspec:scxml-interpretation#Session Termination#Events after termination are rejected]] {level: unit}
#[test]
fn events_after_termination_are_rejected() {
    let cc = compile(chart(vec![final_("end")]));
    let o = run_null(&cc);
    assert!(o.session.is_terminated());
    assert!(matches!(
        o.effects.as_slice(),
        [Effect::SessionDone {
            cancelled: false,
            ..
        }]
    ));
    let r = step(
        &cc,
        o.session,
        Input::event(ev("e"), Environment::default()),
        &mut NullDatamodel,
    );
    assert_eq!(r, Err(StepError::Terminated));
}
