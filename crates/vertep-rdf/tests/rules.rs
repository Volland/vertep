//! Each well-formedness rule of the chart model, broken one at a time in an otherwise
//! well-formed chart: the IR validator and the chart shapes must both reject it.

mod common;

use vertep_core::ir::*;
use vertep_rdf::{project, validate};

fn state<'a>(c: &'a mut Chart, path: &[usize]) -> &'a mut State {
    let mut s = &mut c.states[path[0]];
    for i in &path[1..] {
        s = &mut s.children[*i];
    }
    s
}

#[test]
fn every_rule_rejected_by_both_validators() {
    type Mutation = (&'static str, fn(&mut Chart));
    let mutations: Vec<Mutation> = vec![
        ("empty chart", |c| c.states.clear()),
        ("empty state id", |c| state(c, &[3]).id = String::new()),
        ("duplicate state id", |c| {
            state(c, &[1, 0]).id = "closed".into()
        }),
        ("unknown target", |c| {
            state(c, &[0]).transitions[0].targets = vec!["nowhere".into()]
        }),
        ("unknown root initial", |c| {
            c.initial = vec!["nowhere".into()]
        }),
        ("initial attribute not a descendant", |c| {
            state(c, &[1]).initial = vec!["closed".into()]
        }),
        ("initial attribute and element", |c| {
            let s = state(c, &[2, 1]);
            s.initial = vec!["p.b.1".into()];
        }),
        ("initial on atomic", |c| {
            state(c, &[0]).initial = vec!["closed".into()]
        }),
        ("initial on parallel", |c| {
            state(c, &[2]).initial = vec!["p.a".into()]
        }),
        ("initial transition with event", |c| {
            state(c, &[2, 1])
                .initial_element
                .as_mut()
                .unwrap()
                .transition
                .events = vec!["e".into()];
        }),
        ("initial transition without target", |c| {
            state(c, &[2, 1])
                .initial_element
                .as_mut()
                .unwrap()
                .transition
                .targets
                .clear();
        }),
        ("initial element target outside", |c| {
            state(c, &[2, 1])
                .initial_element
                .as_mut()
                .unwrap()
                .transition
                .targets = vec!["p.a".into()];
        }),
        ("history with two transitions", |c| {
            let h = state(c, &[1, 2]);
            h.transitions.push(h.transitions[0].clone());
        }),
        ("history transition with condition", |c| {
            state(c, &[1, 2]).transitions[0].cond = Some("x".into());
        }),
        ("history transition with event", |c| {
            state(c, &[1, 2]).transitions[0].events = vec!["e".into()];
        }),
        ("history target outside parent", |c| {
            state(c, &[1, 2]).transitions[0].targets = vec!["closed".into()];
        }),
        ("history at root", |c| {
            c.states.push(State::new(
                "hr",
                StateElement::History(HistoryType::Shallow),
            ));
        }),
        ("history with onentry", |c| {
            state(c, &[1, 2]).onentry.push(vec![])
        }),
        ("history with child", |c| {
            state(c, &[1, 2]).children.push(common::atomic("hc"))
        }),
        ("final with transition", |c| {
            state(c, &[3])
                .transitions
                .push(common::transition(&["e"], &["closed"]));
        }),
        ("final with child", |c| {
            state(c, &[3]).children.push(common::atomic("fc"))
        }),
        ("final with data", |c| {
            let d = c.data[0].clone();
            state(c, &[3]).data.push(Data {
                id: "fd".into(),
                ..d
            });
        }),
        ("donedata outside final", |c| {
            let dd = state(c, &[3]).donedata.clone();
            state(c, &[0]).donedata = dd;
        }),
        ("parallel with only history children", |c| {
            let mut p = State::new("ph2", StateElement::Parallel);
            p.children.push(State::new(
                "ph2.h",
                StateElement::History(HistoryType::Deep),
            ));
            c.states.push(p);
        }),
        ("target set in one compound", |c| {
            state(c, &[0]).transitions[0].targets = vec!["o1".into(), "o2".into()];
        }),
        ("target set with ancestor", |c| {
            state(c, &[0]).transitions[0].targets = vec!["p".into(), "p.a".into()];
        }),
        ("history target beside its sibling", |c| {
            state(c, &[0]).transitions[0].targets = vec!["h".into(), "o1".into()];
        }),
        ("history default cycle", |c| {
            let open = state(c, &[1]);
            open.initial = vec!["h".into()];
            open.children[2].transitions.clear();
        }),
        ("history beside a state inside its parent", |c| {
            state(c, &[0]).transitions[0].targets = vec!["ph".into(), "p.a".into()];
        }),
        ("duplicate target", |c| {
            state(c, &[0]).transitions[0].targets = vec!["p.a".into(), "p.a".into()];
        }),
        ("root initial target set", |c| {
            c.initial = vec!["closed".into(), "open".into()]
        }),
        ("duplicate data id", |c| c.data[1].id = "x".into()),
        ("data src and expr", |c| c.data[2].expr = Some("1".into())),
        ("data content and expr", |c| {
            c.data[1].expr = Some("1".into())
        }),
        ("param expr and location", |c| {
            state(c, &[3]).donedata = Some(DoneData {
                content: None,
                params: vec![Param {
                    name: "p".into(),
                    expr: Some("1".into()),
                    location: Some("x".into()),
                    span: Span::NONE,
                }],
                span: Span::NONE,
            });
        }),
        ("donedata content and params", |c| {
            let dd = state(c, &[4]).donedata.clone().unwrap();
            state(c, &[3]).donedata.as_mut().unwrap().params = dd.params;
        }),
        ("send event and eventexpr", |c| {
            if let ExecKind::Send(s) = &mut state(c, &[0]).onexit[0][0].kind {
                s.eventexpr = Some("e".into());
            }
        }),
        ("send content and namelist", |c| {
            if let ExecKind::Send(s) = &mut state(c, &[0]).onexit[0][2].kind {
                s.namelist = vec!["x".into()];
            }
        }),
        ("cancel without sendid", |c| {
            state(c, &[0]).onexit[0][3].kind = ExecKind::Cancel {
                sendid: None,
                sendidexpr: None,
            };
        }),
        ("assign expr and content", |c| {
            if let ExecKind::Assign { expr, .. } = &mut state(c, &[0]).transitions[0].body[1].kind {
                *expr = Some("1".into());
            }
        }),
        ("else before elseif", |c| {
            let inv = &mut state(c, &[1, 1]).invokes[0];
            if let ExecKind::Foreach { body, .. } = &mut inv.finalize[0].kind
                && let ExecKind::If { branches } = &mut body[0].kind
            {
                branches.swap(1, 2);
            }
        }),
        ("first branch without condition", |c| {
            state(c, &[0]).onentry[0].push(Exec::new(ExecKind::If {
                branches: vec![IfBranch {
                    cond: None,
                    body: vec![],
                    span: Span::NONE,
                }],
            }));
        }),
        ("foreach without item", |c| {
            if let ExecKind::Foreach { item, .. } =
                &mut state(c, &[1, 1]).invokes[0].finalize[0].kind
            {
                item.clear();
            }
        }),
        ("invoke type and typeexpr", |c| {
            state(c, &[1, 1]).invokes[0].typeexpr = Some("t".into());
        }),
        ("invoke content and srcexpr", |c| {
            state(c, &[1, 1]).invokes[1].srcexpr = Some("x".into());
        }),
        ("invoke content expr and src", |c| {
            let inv = &mut state(c, &[1, 1]).invokes[1];
            inv.src = Some("x.scxml".into());
            inv.content = Some(Content {
                expr: Some("doc".into()),
                body: None,
                span: Span::NONE,
            });
        }),
        ("state with only history children", |c| {
            let mut s = common::atomic("sh");
            s.children
                .push(State::new("sh.h", StateElement::History(HistoryType::Deep)));
            c.states.push(s);
        }),
        ("invoke params and namelist", |c| {
            state(c, &[1, 1]).invokes[0].namelist = vec!["x".into()];
        }),
    ];
    let base = common::kitchen_sink();
    assert_eq!(vertep_core::validate::validate(&base), Ok(()));
    assert!(validate(&project(&base).graph).conforms());
    let mut disagreements = Vec::new();
    for (name, mutate) in mutations {
        let mut c = base.clone();
        mutate(&mut c);
        assert_ne!(c, base, "{name}: mutation changed nothing");
        let ir = vertep_core::validate::validate(&c).is_ok();
        let shapes = validate(&project(&c).graph).conforms();
        if ir || shapes {
            disagreements.push(format!("{name}: IR accepts {ir}, shapes accept {shapes}"));
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}

#[test]
fn legal_variants_accepted_by_both_validators() {
    type Mutation = (&'static str, fn(&mut Chart));
    let mutations: Vec<Mutation> = vec![
        ("transition to a history state", |c| {
            state(c, &[0]).transitions[0].targets = vec!["h".into()];
        }),
        ("history of a parallel state", |c| {
            state(c, &[0]).transitions[0].targets = vec!["ph".into()];
        }),
        ("targets in two regions", |c| {
            state(c, &[0]).transitions[0].targets = vec!["p.b.1".into(), "p.a".into()];
        }),
        ("history defaulting to its parent's initial", |c| {
            state(c, &[1, 2]).transitions.clear();
        }),
        ("data from src with resolved content", |c| {
            c.data[2].content = Some("{}".into());
        }),
        ("invoke src with resolved content", |c| {
            state(c, &[1, 1]).invokes[1].src = Some("x.scxml".into());
        }),
    ];
    let base = common::kitchen_sink();
    let mut disagreements = Vec::new();
    for (name, mutate) in mutations {
        let mut c = base.clone();
        mutate(&mut c);
        let ir = vertep_core::validate::validate(&c);
        let report = validate(&project(&c).graph);
        if ir.is_err() || !report.conforms() {
            disagreements.push(format!("{name}: IR {ir:?}, shapes {report}"));
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}
