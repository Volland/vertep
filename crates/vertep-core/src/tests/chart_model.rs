//! One test per chart-model scenario.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::builder::*;
use crate::compile::{CompiledChart, Domain};
use crate::identity::{GENERATED_PREFIX, assign_generated_ids};
use crate::ir::*;
use crate::order::{DocNode, document_order, same_chart};
use crate::session::Effect;
use crate::validate::{
    InitialSource, NodeRef, ViolationKind, resolve_initial, resolve_root_initial, validate,
};

// @tg: verifies:: [[openspec:chart-model#Chart Document Structure#Root with nested states]] {level: unit}
#[test]
fn root_with_nested_states() {
    let c = chart(vec![
        compound("a", vec![atomic("a1"), atomic("a2")]),
        final_("done"),
    ]);
    assert_eq!(c.states.len(), 2);
    assert_eq!(c.states[0].id, "a");
    assert_eq!(c.states[1].id, "done");
    assert_eq!(c.states[0].kind(), StateKind::Compound);
    assert_eq!(c.states[0].children[0].kind(), StateKind::Atomic);
    assert_eq!(c.states[0].children[1].kind(), StateKind::Atomic);
    assert_eq!(c.states[1].kind(), StateKind::Final);
    let cc = compile(c);
    let order: Vec<&str> = cc.states()[1..].iter().map(|s| s.id.as_str()).collect();
    assert_eq!(order, vec!["a", "a1", "a2", "done"]);
    assert_eq!(
        cc.state(cc.index_of("a1").unwrap()).parent,
        cc.index_of("a")
    );
}

// @tg: verifies:: [[openspec:chart-model#Chart Document Structure#Every SCXML element kind is representable]] {level: unit}
#[test]
fn every_scxml_element_kind_is_representable() {
    let content = Content {
        expr: None,
        body: Some("<scxml/>".into()),
        span: Span::NONE,
    };
    let param = Param {
        name: "p".into(),
        expr: Some("1".into()),
        location: None,
        span: Span::NONE,
    };
    let data = Data {
        id: "x".into(),
        src: None,
        expr: Some("1".into()),
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    };
    let execs = vec![
        raise("r"),
        send(Send {
            event: Some("e".into()),
            params: vec![param.clone()],
            ..Send::default()
        }),
        Exec::new(ExecKind::Cancel {
            sendid: Some("s".into()),
            sendidexpr: None,
        }),
        log("l"),
        assign("x", "2"),
        Exec::new(ExecKind::Script(Script {
            src: None,
            body: Some("x".into()),
            span: Span::NONE,
        })),
        if_(vec![(Some("c"), vec![]), (None, vec![])]),
        Exec::new(ExecKind::Foreach {
            array: "arr".into(),
            item: "it".into(),
            index: Some("i".into()),
            body: vec![],
        }),
    ];
    let mut s = compound(
        "s",
        vec![
            history("h", HistoryType::Deep, "s1"),
            atomic("s1"),
            parallel("p", vec![atomic("p1"), atomic("p2")]),
            final_("f"),
        ],
    )
    .entry(execs.clone())
    .exit(vec![log("x")])
    .on(tr("go", "f").body(execs.clone()))
    .invoke(Invoke {
        type_: Some("scxml".into()),
        content: Some(content.clone()),
        params: vec![param.clone()],
        finalize: vec![log("fin")],
        ..Invoke::default()
    });
    s.initial_element = Some(Initial {
        transition: tr("", "s1"),
        extensions: Vec::new(),
        span: Span::NONE,
    });
    s.data.push(data);
    s.children[3].donedata = Some(DoneData {
        content: None,
        params: vec![param],
        span: Span::NONE,
    });
    let c = chart(vec![s]);
    assert_eq!(validate(&c), Ok(()));

    // Every state element has its own kind.
    let s = &c.states[0];
    let kinds: Vec<StateKind> = s.children.iter().map(State::kind).collect();
    assert_eq!(
        kinds,
        vec![
            StateKind::History(HistoryType::Deep),
            StateKind::Atomic,
            StateKind::Parallel,
            StateKind::Final
        ]
    );
    assert_eq!(s.kind(), StateKind::Compound);
    assert!(s.initial_element.is_some());
    assert_eq!(s.transitions[0].targets, vec!["f"]);
    assert_eq!(s.onentry.len(), 1);
    assert_eq!(s.onexit.len(), 1);
    assert_eq!(s.data[0].id, "x");
    assert!(s.children[3].donedata.is_some());
    assert_eq!(s.invokes[0].content, Some(content));
    assert_eq!(s.invokes[0].finalize.len(), 1);
    // Every executable content element has a distinct representation.
    let discr: Vec<_> = s.onentry[0]
        .iter()
        .map(|e| core::mem::discriminant(&e.kind))
        .collect();
    for i in 0..discr.len() {
        for j in (i + 1)..discr.len() {
            assert_ne!(discr[i], discr[j]);
        }
    }
    assert_eq!(discr.len(), 8);
    // And the whole chart compiles.
    assert!(CompiledChart::new(c).is_ok());
}

// @tg: verifies:: [[openspec:chart-model#State Kinds#Compound versus atomic]] {level: unit}
#[test]
fn compound_versus_atomic() {
    for child in [atomic("c"), parallel("c", vec![atomic("c1")]), final_("c")] {
        assert_eq!(compound("s", vec![child]).kind(), StateKind::Compound);
    }
    let mut a = atomic("s").on(tr("e", "s"));
    a.onentry.push(vec![log("x")]);
    assert_eq!(a.kind(), StateKind::Atomic);
    // The compiled chart agrees: the atomic state is a selection leaf.
    let cc = compile(chart(vec![compound("s", vec![atomic("c")])]));
    let c = cc.index_of("c").unwrap();
    assert!(crate::kernel::is_atomic(&cc, c));
    assert!(!crate::kernel::is_atomic(&cc, cc.index_of("s").unwrap()));
}

// @tg: verifies:: [[openspec:chart-model#State Kinds#History type defaults to shallow]] {level: unit}
#[test]
fn history_type_defaults_to_shallow() {
    assert_eq!(HistoryType::default(), HistoryType::Shallow);
    let h = State::new("h", StateElement::History(HistoryType::default()));
    assert_eq!(h.kind(), StateKind::History(HistoryType::Shallow));
}

fn anonymous_chart() -> Chart {
    chart(vec![
        compound("", vec![atomic(""), atomic("named")]),
        atomic(""),
    ])
}

// @tg: verifies:: [[openspec:chart-model#State Identity#Generated identifiers are stable]] {level: unit}
#[test]
fn generated_identifiers_are_stable() {
    let mut first = anonymous_chart();
    let mut second = anonymous_chart();
    assign_generated_ids(&mut first);
    assign_generated_ids(&mut second);
    assert_eq!(first, second);
    assert_eq!(first.states[0].id, "_vt.s.0");
    assert_eq!(first.states[0].children[0].id, "_vt.s.0.0");
    assert_eq!(first.states[1].id, "_vt.s.1");
    assert!(first.states[0].id_generated);
    assert!(!first.states[0].children[1].id_generated);
    assert_eq!(validate(&first), Ok(()));
}

// @tg: verifies:: [[openspec:chart-model#State Identity#Generated identifiers avoid declared names]] {level: unit}
#[test]
fn generated_identifiers_avoid_declared_names() {
    let collide = alloc::format!("{GENERATED_PREFIX}.1");
    let mut c = chart(vec![compound(&collide, vec![atomic("x")]), atomic("")]);
    assign_generated_ids(&mut c);
    let generated = &c.states[1].id;
    assert!(c.states[1].id_generated);
    assert_ne!(generated, &collide);
    assert_ne!(generated, "x");
    assert_eq!(validate(&c), Ok(()));
}

// @tg: verifies:: [[openspec:chart-model#Document Order#Transition order preserved]] {level: unit}
#[test]
fn transition_order_preserved() {
    let c = chart(vec![
        atomic("s")
            .on(tr("t1", "s"))
            .on(tr("t2", "s"))
            .on(tr("t3", "s")),
    ]);
    let names: Vec<&str> = c.states[0]
        .transitions
        .iter()
        .map(|t| t.events[0].as_str())
        .collect();
    assert_eq!(names, vec!["t1", "t2", "t3"]);
    let positions: Vec<(usize, usize)> = document_order(&c)
        .iter()
        .enumerate()
        .filter_map(|(pos, n)| match n {
            DocNode::Transition { index, .. } => Some((*index, pos)),
            _ => None,
        })
        .collect();
    assert_eq!(positions.len(), 3);
    assert_eq!(positions[0].0, 0);
    assert_eq!(positions[1].0, 1);
    assert_eq!(positions[2].0, 2);
    assert!(positions[0].1 < positions[1].1 && positions[1].1 < positions[2].1);
    // The compiled chart keeps the order too.
    let cc = compile(c);
    let ts = &cc.state(cc.index_of("s").unwrap()).transitions;
    assert!(ts[0] < ts[1] && ts[1] < ts[2]);
}

// @tg: verifies:: [[openspec:chart-model#Document Order#Multiple targets keep their order]] {level: unit}
#[test]
fn multiple_targets_keep_their_order() {
    let c = chart(vec![
        atomic("s").on(tr("go", "p.b p.a")),
        parallel("p", vec![atomic("p.a"), atomic("p.b")]),
    ]);
    assert_eq!(c.states[0].transitions[0].targets, vec!["p.b", "p.a"]);
    let cc = compile(c);
    let t = cc.state(cc.index_of("s").unwrap()).transitions[0];
    let targets: Vec<&str> = cc
        .transition(t)
        .targets
        .iter()
        .map(|i| cc.state(*i).id.as_str())
        .collect();
    assert_eq!(targets, vec!["p.b", "p.a"]);
}

// @tg: verifies:: [[openspec:chart-model#Transition Structure#Targetless transition]] {level: unit}
#[test]
fn targetless_transition() {
    let c = chart(vec![
        atomic("s")
            .logged()
            .on(tr("ping", "").body(vec![log("pong")])),
    ]);
    let t = &c.states[0].transitions[0];
    assert!(t.targets.is_empty());
    assert_eq!(t.events, vec!["ping"]);
    let cc = compile(c);
    let ti = cc.state(cc.index_of("s").unwrap()).transitions[0];
    assert_eq!(cc.transition(ti).domain, Domain::Targetless);
    // Taking it runs its content and exits nothing.
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "ping");
    assert_eq!(logs(&o.effects), ids(&["pong"]));
    assert_eq!(active(&cc, &o), ids(&["s"]));
}

// @tg: verifies:: [[openspec:chart-model#Transition Structure#Eventless transition]] {level: unit}
#[test]
fn eventless_transition() {
    let c = chart(vec![atomic("s").on(tr("", "t")), atomic("t")]);
    assert!(c.states[0].transitions[0].events.is_empty());
    let cc = compile(c);
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["t"]));
    assert_eq!(o.trace.microsteps[0].event, None);
}

// @tg: verifies:: [[openspec:chart-model#Initial State Resolution#Initial attribute wins]] {level: unit}
#[test]
fn initial_attribute_wins() {
    let s = compound("c", vec![atomic("s1"), atomic("s2")]).initial("s2");
    let r = resolve_initial(&s);
    assert_eq!(r.targets, vec!["s2"]);
    assert_eq!(r.source, InitialSource::Attribute);
    let cc = compile(chart(vec![s]));
    let c = cc.index_of("c").unwrap();
    assert_eq!(cc.default_entry(c), &[cc.index_of("s2").unwrap()]);
    assert_eq!(active(&cc, &run_null(&cc)), ids(&["c", "s2"]));
}

// @tg: verifies:: [[openspec:chart-model#Initial State Resolution#First child by default]] {level: unit}
#[test]
fn first_child_by_default() {
    let s = compound(
        "c",
        vec![
            history("h", HistoryType::Shallow, "s2"),
            atomic("s1"),
            atomic("s2"),
        ],
    );
    let r = resolve_initial(&s);
    assert_eq!(r.targets, vec!["s1"]);
    assert_eq!(r.source, InitialSource::FirstChild);
    let c = chart(vec![s, atomic("other")]);
    assert_eq!(resolve_root_initial(&c).targets, vec!["c"]);
    let cc = compile(c);
    assert_eq!(active(&cc, &run_null(&cc)), ids(&["c", "s1"]));
}

// @tg: verifies:: [[openspec:chart-model#Executable Content Model#Send with expressions]] {level: unit}
#[test]
fn send_with_expressions() {
    let s = Send {
        eventexpr: Some("'ev' + n".into()),
        targetexpr: Some("dest".into()),
        delay: Some("2s".into()),
        namelist: vec!["a".into(), "b".into()],
        ..Send::default()
    };
    let mut c = chart(vec![atomic("s").entry(vec![send(s.clone())])]);
    c.datamodel = "ecmascript".into();
    c.datamodel_explicit = true;
    assert_eq!(validate(&c), Ok(()));
    let ExecKind::Send(kept) = &c.states[0].onentry[0][0].kind else {
        panic!("not a send")
    };
    assert_eq!(kept.eventexpr.as_deref(), Some("'ev' + n"));
    assert_eq!(kept.targetexpr.as_deref(), Some("dest"));
    assert_eq!(kept.delay.as_deref(), Some("2s"));
    assert_eq!(kept.namelist, vec!["a", "b"]);
    assert_eq!(c.datamodel, "ecmascript");
    // Compiling does not evaluate anything: the text is still there.
    let cc = compile(c);
    assert_eq!(cc.datamodel(), "ecmascript");
    let st = cc.state(cc.index_of("s").unwrap());
    assert_eq!(st.onentry[0][0].kind, ExecKind::Send(Box::new(s)));
}

// @tg: verifies:: [[openspec:chart-model#Executable Content Model#Conditional branches]] {level: unit}
#[test]
fn conditional_branches() {
    let e = if_(vec![
        (Some("In('nowhere')"), vec![log("if")]),
        (Some("In('s')"), vec![log("elseif")]),
        (None, vec![log("else")]),
    ]);
    let ExecKind::If { branches } = &e.kind else {
        panic!()
    };
    assert_eq!(branches.len(), 3);
    assert!(branches[0].cond.is_some() && branches[1].cond.is_some());
    assert!(branches[2].cond.is_none());
    let cc = compile(chart(vec![atomic("s").entry(vec![e])]));
    assert_eq!(logs(&run_null(&cc).effects), ids(&["elseif"]));
}

// @tg: verifies:: [[openspec:chart-model#Invoke Declaration#Invoke with inline content]] {level: unit}
#[test]
fn invoke_with_inline_content() {
    let inv = Invoke {
        type_: Some("scxml".into()),
        autoforward: true,
        content: Some(Content {
            expr: None,
            body: Some("<scxml initial=\"c\"><final id=\"c\"/></scxml>".into()),
            span: Span::NONE,
        }),
        ..Invoke::default()
    };
    let cc = compile(chart(vec![atomic("s").invoke(inv)]));
    let st = cc.state(cc.index_of("s").unwrap());
    assert_eq!(st.invokes[0].type_.as_deref(), Some("scxml"));
    assert!(st.invokes[0].autoforward);
    assert!(st.invokes[0].content.as_ref().unwrap().body.is_some());
    let o = run_null(&cc);
    let Some(Effect::Invoke(req)) = o.effects.last() else {
        panic!("no invoke effect")
    };
    assert!(req.autoforward);
    assert_eq!(
        req.content,
        Some(crate::datamodel::Value::String(
            "<scxml initial=\"c\"><final id=\"c\"/></scxml>".into()
        ))
    );
}

// @tg: verifies:: [[openspec:chart-model#Invoke Declaration#Generated invoke identifier location]] {level: unit}
#[test]
fn generated_invoke_identifier_location() {
    let inv = Invoke {
        idlocation: Some("where".into()),
        ..Invoke::default()
    };
    let mut c = chart(vec![atomic("s").invoke(inv)]);
    c.datamodel = "mock".into();
    c.data.push(Data {
        id: "where".into(),
        src: None,
        expr: None,
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    });
    let st = &c.states[0].invokes[0];
    assert_eq!(st.idlocation.as_deref(), Some("where"));
    assert!(st.id.is_none());
    let cc = compile(c);
    let mut dm = MockDm::named("mock");
    let o = crate::session::start(&cc, start_input(), &mut dm).unwrap();
    let Some(Effect::Invoke(req)) = o.effects.last() else {
        panic!("no invoke effect")
    };
    assert!(req.invokeid.starts_with("s."));
    assert_eq!(
        dm.vars.get("where"),
        Some(&crate::datamodel::Value::String(req.invokeid.clone()))
    );
}

// @tg: verifies:: [[openspec:chart-model#Datamodel Declaration#Data declared in a nested state]] {level: unit}
#[test]
fn data_declared_in_a_nested_state() {
    let mut s = atomic("s");
    s.data.push(Data {
        id: "x".into(),
        src: None,
        expr: Some("1".into()),
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    });
    let cc = compile(chart(vec![compound("outer", vec![s])]));
    let owner = cc
        .states()
        .iter()
        .find(|st| st.data.iter().any(|d| d.id == "x"))
        .unwrap();
    assert_eq!(owner.id, "s");
    assert_eq!(owner.data[0].expr.as_deref(), Some("1"));
}

// @tg: verifies:: [[openspec:chart-model#Datamodel Declaration#Late binding recorded]] {level: unit}
#[test]
fn late_binding_recorded() {
    let mut c = chart(vec![atomic("s")]);
    c.binding = Binding::Late;
    c.binding_explicit = true;
    assert_eq!(c.binding, Binding::Late);
    let cc = compile(c);
    assert_eq!(cc.binding(), Binding::Late);
}

// @tg: verifies:: [[openspec:chart-model#Extension Nodes#Vertep extension attribute]] {level: unit}
#[test]
fn vertep_extension_attribute() {
    let mut s = atomic("s");
    s.extensions.push(Extension {
        ns: VT_NS.into(),
        local: "role".into(),
        value: ExtensionValue::Attribute("decider".into()),
        position: 0,
        span: Span::NONE,
    });
    let ext = &s.extensions[0];
    assert_eq!(ext.ns, "https://w3id.org/vertep/vt#");
    assert_eq!(ext.local, "role");
    assert_eq!(ext.value, ExtensionValue::Attribute("decider".into()));
    // The extension is part of the chart: it takes part in equality.
    assert_ne!(chart(vec![s.clone()]), chart(vec![atomic("s")]));
    assert!(CompiledChart::new(chart(vec![s])).is_ok());
}

// @tg: verifies:: [[openspec:chart-model#Extension Nodes#Foreign element preserved]] {level: unit}
#[test]
fn foreign_element_preserved() {
    let foreign = Extension {
        ns: "urn:example:other".into(),
        local: "note".into(),
        value: ExtensionValue::Element("<o:note xmlns:o=\"urn:example:other\">hi</o:note>".into()),
        position: 1,
        span: Span::new(3, 5),
    };
    let mut t = tr("e", "s").body(vec![
        log("before"),
        Exec::new(ExecKind::Extension(foreign.clone())),
        log("after"),
    ]);
    t.extensions.push(foreign.clone());
    let c = chart(vec![atomic("s").on(t)]);
    let kept = &c.states[0].transitions[0];
    assert_eq!(kept.extensions[0], foreign);
    assert_eq!(kept.extensions[0].position, 1);
    assert_eq!(kept.body[1].kind, ExecKind::Extension(foreign));
    // Unknown executable content is skipped at run time; its neighbours still run in order.
    let cc = compile(c);
    let o = run_null(&cc);
    let o = send_null(&cc, o.session, "e");
    assert_eq!(logs(&o.effects), ids(&["before", "after"]));
}

// @tg: verifies:: [[openspec:chart-model#Well-Formedness Validation#Unknown target rejected]] {level: unit}
#[test]
fn unknown_target_rejected() {
    let c = chart(vec![atomic("s").on(tr("e", "s")).on(tr("go", "nowhere"))]);
    let errs = validate(&c).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(
        errs[0].kind,
        ViolationKind::UnknownTarget {
            target: "nowhere".into()
        }
    );
    assert_eq!(
        errs[0].node,
        NodeRef::Transition {
            source: "s".into(),
            index: 1
        }
    );
    assert!(CompiledChart::new(c).is_err());
}

// @tg: verifies:: [[openspec:chart-model#Well-Formedness Validation#Several violations reported together]] {level: unit}
#[test]
fn several_violations_reported_together() {
    let c = chart(vec![
        compound("a", vec![atomic("a1")]).initial("b"),
        atomic("b"),
        atomic("b"),
    ]);
    let errs = validate(&c).unwrap_err();
    let kinds: Vec<&ViolationKind> = errs.iter().map(|v| &v.kind).collect();
    assert!(kinds.contains(&&ViolationKind::DuplicateStateId { id: "b".into() }));
    assert!(kinds.contains(&&ViolationKind::IllegalInitialTarget { target: "b".into() }));
    assert_eq!(errs.len(), 2);
}

// @tg: verifies:: [[openspec:chart-model#Well-Formedness Validation#Valid chart accepted]] {level: unit}
#[test]
fn valid_chart_accepted() {
    let c = chart(vec![
        compound(
            "a",
            vec![
                history("h", HistoryType::Shallow, "a1"),
                atomic("a1").on(tr("e", "a2")),
                atomic("a2"),
            ],
        )
        .on(tr("done", "end")),
        parallel("p", vec![atomic("p1"), atomic("p2")]),
        final_("end"),
    ]);
    assert_eq!(validate(&c), Ok(()));
    let cc = CompiledChart::new(c).expect("compiles");
    let o = run_null(&cc);
    assert_eq!(active(&cc, &o), ids(&["a", "a1"]));
}

/// More rules: each is reported with its node.
#[test]
fn other_well_formedness_rules() {
    let mut bad_history = history("h", HistoryType::Shallow, "x");
    bad_history.transitions[0].events = vec!["e".into()];
    let c = chart(vec![
        compound("a", vec![bad_history, atomic("a1")]),
        atomic("x"),
        final_("f").on(tr("e", "x")),
        parallel("p", vec![history("ph", HistoryType::Deep, "p")]),
        atomic("s").on(tr("e", "a1 x")),
        atomic("t").entry(vec![Exec::new(ExecKind::Cancel {
            sendid: None,
            sendidexpr: None,
        })]),
    ]);
    let errs = validate(&c).unwrap_err();
    let kinds: Vec<ViolationKind> = errs.iter().map(|v| v.kind.clone()).collect();
    assert!(kinds.contains(&ViolationKind::IllegalHistoryTransition));
    assert!(kinds.contains(&ViolationKind::IllegalHistoryTarget { target: "x".into() }));
    assert!(kinds.contains(&ViolationKind::FinalContent {
        what: "transitions".into()
    }));
    assert!(kinds.contains(&ViolationKind::ParallelHistoryOnly));
    assert!(kinds.contains(&ViolationKind::IllegalTargetSet {
        first: "a1".into(),
        second: "x".into()
    }));
    assert!(kinds.contains(&ViolationKind::ConflictingAttributes {
        element: "cancel".into(),
        attributes: "sendid, sendidexpr".into()
    }));
    assert!(validate(&chart(vec![])).is_err());
    let missing = chart(vec![atomic("")]);
    assert_eq!(
        validate(&missing).unwrap_err()[0].kind,
        ViolationKind::MissingId
    );
}

fn equality_chart(swap: bool, line: u32) -> Chart {
    let mut t1 = tr("e1", "b");
    let mut t2 = tr("e2", "b");
    t1.span = Span::new(line, 1);
    t2.span = Span::new(line + 1, 1);
    let ts = if swap { vec![t2, t1] } else { vec![t1, t2] };
    let mut a = atomic("a");
    a.transitions = ts;
    a.span = Span::new(line, 0);
    chart(vec![a, atomic("b")])
}

// @tg: verifies:: [[openspec:chart-model#Chart Equality#Order matters for equality]] {level: unit}
#[test]
fn order_matters_for_equality() {
    assert!(!same_chart(
        &equality_chart(false, 1),
        &equality_chart(true, 1)
    ));
    assert!(same_chart(
        &equality_chart(false, 1),
        &equality_chart(false, 1)
    ));
    // Expression text and extensions count too.
    let mut changed = equality_chart(false, 1);
    changed.states[0].transitions[0].cond = Some("In('b')".into());
    assert!(!same_chart(&equality_chart(false, 1), &changed));
}

// @tg: verifies:: [[openspec:chart-model#Chart Equality#Equal after lowering the same source twice]] {level: unit}
#[test]
fn equal_after_lowering_the_same_source_twice() {
    // Two lowerings of the same source; source locations differ but are not compared.
    let mut first = equality_chart(false, 1);
    let mut second = equality_chart(false, 40);
    assign_generated_ids(&mut first);
    assign_generated_ids(&mut second);
    assert!(same_chart(&first, &second));
    // Spans are still carried, they just do not take part in equality.
    assert_ne!(first.states[0].span.line, second.states[0].span.line);
    let a = compile(first);
    let b = compile(second);
    assert_eq!(a.chart(), b.chart());
}
