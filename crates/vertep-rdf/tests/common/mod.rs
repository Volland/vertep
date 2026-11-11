//! Charts shared by the integration tests.
#![allow(dead_code, missing_docs)]

use vertep_core::ir::*;

pub fn ext_attr(local: &str, value: &str) -> Extension {
    Extension {
        ns: VT_NS.into(),
        local: local.into(),
        value: ExtensionValue::Attribute(value.into()),
        position: 0,
        span: Span::NONE,
    }
}

pub fn ext_elem(ns: &str, local: &str, xml: &str, position: u32) -> Extension {
    Extension {
        ns: ns.into(),
        local: local.into(),
        value: ExtensionValue::Element(xml.into()),
        position,
        span: Span::NONE,
    }
}

pub fn transition(events: &[&str], targets: &[&str]) -> Transition {
    Transition {
        events: events.iter().map(|s| s.to_string()).collect(),
        targets: targets.iter().map(|s| s.to_string()).collect(),
        ..Transition::default()
    }
}

pub fn atomic(id: &str) -> State {
    State::new(id, StateElement::State)
}

pub fn compound(id: &str, children: Vec<State>) -> State {
    let mut s = State::new(id, StateElement::State);
    s.children = children;
    s
}

pub fn chart(datamodel: &str, states: Vec<State>) -> Chart {
    Chart {
        datamodel: datamodel.into(),
        datamodel_explicit: true,
        states,
        ..Chart::default()
    }
}

/// Two states and one guarded transition, as in the RDF view.
pub fn door() -> Chart {
    let mut closed = atomic("closed");
    let mut t = transition(&["door.open"], &["open"]);
    t.cond = Some("!locked".into());
    closed.transitions.push(t);
    let mut c = chart("ecmascript", vec![closed, atomic("open")]);
    c.name = Some("door".into());
    c
}

/// A well-formed chart that uses every IR node kind and attribute.
pub fn kitchen_sink() -> Chart {
    let raise = |e: &str| Exec::new(ExecKind::Raise { event: e.into() });

    // closed: onentry, two transitions, onexit with send and cancel.
    let mut closed = atomic("closed");
    closed.extensions.push(ext_attr("color", "red"));
    closed.onentry.push(vec![
        Exec::new(ExecKind::Log {
            label: Some("in".into()),
            expr: Some("x".into()),
        }),
        raise("e"),
    ]);
    closed.onentry.push(vec![Exec::new(ExecKind::Script(Script {
        src: None,
        body: Some("x = x * 2;".into()),
        span: Span::NONE,
    }))]);
    let mut t0 = transition(&["door.open", "go.*"], &["open"]);
    t0.cond = Some("x > 1 && !locked".into());
    t0.kind = TransitionType::Internal;
    t0.kind_explicit = true;
    t0.body.push(Exec::new(ExecKind::Assign {
        location: "x".into(),
        expr: Some("x + 1".into()),
        content: None,
    }));
    t0.body.push(Exec::new(ExecKind::Assign {
        location: "y".into(),
        expr: None,
        content: Some("<point xmlns=\"urn:p\" x=\"1\"/>\n  text & more".into()),
    }));
    let mut ext_exec = Exec::new(ExecKind::Extension(ext_elem(
        "urn:foreign",
        "audit",
        "<f:audit xmlns:f=\"urn:foreign\" level=\"2\"/>",
        2,
    )));
    ext_exec.extensions.push(ext_attr("trace", "on"));
    t0.body.push(ext_exec);
    t0.extensions.push(ext_elem(
        "urn:foreign",
        "note",
        "<f:note xmlns:f=\"urn:foreign\">hi</f:note>",
        1,
    ));
    closed.transitions.push(t0);
    closed.transitions.push(transition(&["error.*"], &["p"]));
    closed.transitions.push(transition(&["tick"], &[]));
    closed.onexit.push(vec![
        Exec::new(ExecKind::Send(Box::new(Send {
            event: Some("s".into()),
            target: Some("#_internal".into()),
            delay: Some("1s".into()),
            id: Some("send-1".into()),
            namelist: vec!["x".into(), "y".into()],
            ..Send::default()
        }))),
        Exec::new(ExecKind::Send(Box::new(Send {
            eventexpr: Some("'ev.' + x".into()),
            targetexpr: Some("t".into()),
            typeexpr: Some("ty".into()),
            idlocation: Some("loc".into()),
            delayexpr: Some("'2s'".into()),
            params: vec![Param {
                name: "a".into(),
                expr: Some("1".into()),
                location: None,
                span: Span::NONE,
            }],
            ..Send::default()
        }))),
        Exec::new(ExecKind::Send(Box::new(Send {
            event: Some("c".into()),
            type_: Some("http://www.w3.org/TR/scxml/#SCXMLEventProcessor".into()),
            content: Some(Content {
                expr: None,
                body: Some("<payload/>".into()),
                span: Span::NONE,
            }),
            ..Send::default()
        }))),
        Exec::new(ExecKind::Cancel {
            sendid: Some("send-1".into()),
            sendidexpr: None,
        }),
        Exec::new(ExecKind::Cancel {
            sendid: None,
            sendidexpr: Some("sid".into()),
        }),
    ]);

    // open: compound with initial attribute, an invoke and a deep history.
    let mut o2 = atomic("o2");
    o2.invokes.push(Invoke {
        type_: Some("scxml".into()),
        id: Some("i1".into()),
        autoforward: true,
        params: vec![Param {
            name: "p".into(),
            expr: None,
            location: Some("x".into()),
            span: Span::NONE,
        }],
        finalize: vec![Exec::new(ExecKind::Foreach {
            array: "arr".into(),
            item: "it".into(),
            index: Some("ix".into()),
            body: vec![Exec::new(ExecKind::If {
                branches: vec![
                    IfBranch {
                        cond: Some("it > 1".into()),
                        body: vec![raise("big")],
                        span: Span::NONE,
                    },
                    IfBranch {
                        cond: Some("it == 1".into()),
                        body: vec![],
                        span: Span::NONE,
                    },
                    IfBranch {
                        cond: None,
                        body: vec![raise("small"), raise("tiny")],
                        span: Span::NONE,
                    },
                ],
            })],
        })],
        extensions: vec![ext_attr("retry", "3")],
        ..Invoke::default()
    });
    o2.invokes.push(Invoke {
        typeexpr: Some("kind".into()),
        content: Some(Content {
            expr: None,
            body: Some("<scxml xmlns=\"http://www.w3.org/2005/07/scxml\" version=\"1.0\"><final id=\"f\"/></scxml>".into()),
            span: Span::NONE,
        }),
        idlocation: Some("iid".into()),
        namelist: vec!["x".into()],
        ..Invoke::default()
    });
    let mut h = State::new("h", StateElement::History(HistoryType::Deep));
    h.transitions.push(transition(&[], &["o1"]));
    let mut open = compound("open", vec![atomic("o1"), o2, h]);
    open.initial = vec!["o2".into()];
    open.data.push(Data {
        id: "local".into(),
        src: None,
        expr: Some("[1, 2]".into()),
        content: None,
        extensions: vec![],
        span: Span::NONE,
    });

    // p: parallel with an <initial> element in one region and a multi-target transition.
    let mut pb = compound("p.b", vec![atomic("p.b.1"), atomic("p.b.2")]);
    let mut init_t = transition(&[], &["p.b.2"]);
    init_t.body.push(raise("init"));
    pb.initial_element = Some(Initial {
        transition: init_t,
        extensions: vec![ext_attr("why", "default")],
        span: Span::NONE,
    });
    let mut p = State::new("p", StateElement::Parallel);
    p.children = vec![atomic("p.a"), pb];
    p.transitions.push(transition(&["t"], &["p.b", "p.a"]));
    let mut shallow = State::new("ph", StateElement::History(HistoryType::Shallow));
    shallow.id_generated = true;
    p.children.push(shallow);

    // done: final with donedata.
    let mut done = State::new("done", StateElement::Final);
    done.donedata = Some(DoneData {
        content: Some(Content {
            expr: Some("x".into()),
            body: None,
            span: Span::NONE,
        }),
        params: vec![],
        span: Span::NONE,
    });
    done.onentry.push(vec![raise("bye")]);
    let mut done2 = State::new("done2", StateElement::Final);
    done2.donedata = Some(DoneData {
        content: None,
        params: vec![Param {
            name: "r".into(),
            expr: Some("x".into()),
            location: None,
            span: Span::NONE,
        }],
        span: Span::NONE,
    });

    let mut c = chart("ecmascript", vec![closed, open, p, done, done2]);
    c.name = Some("kitchen sink".into());
    c.binding = Binding::Late;
    c.binding_explicit = true;
    c.initial = vec!["closed".into()];
    c.data = vec![
        Data {
            id: "x".into(),
            src: None,
            expr: Some("1".into()),
            content: None,
            extensions: vec![],
            span: Span::NONE,
        },
        Data {
            id: "y".into(),
            src: None,
            expr: None,
            content: Some("\n    <items xmlns=\"\">\t<i n=\"1\"/>&amp;</items>\r\n  ".into()),
            extensions: vec![ext_attr("kind", "xml")],
            span: Span::NONE,
        },
        Data {
            id: "z".into(),
            src: Some("data.json".into()),
            expr: None,
            content: None,
            extensions: vec![],
            span: Span::NONE,
        },
    ];
    c.script = Some(Script {
        src: Some("lib.js".into()),
        body: Some("function f() { return \"q\\n\"; }".into()),
        span: Span::NONE,
    });
    c.extensions = vec![
        ext_attr("version", "2"),
        ext_elem(
            VT_NS,
            "meta",
            "<vt:meta xmlns:vt=\"https://w3id.org/vertep/vt#\"/>",
            3,
        ),
    ];
    c
}

/// Give every node a distinct non-trivial span, recursively, so tests can check spans are
/// ignored.
pub fn with_spans(mut c: Chart, line: u32) -> Chart {
    fn state(s: &mut State, line: u32) {
        s.span = Span::new(line, 3);
        for t in &mut s.transitions {
            t.span = Span::new(line + 1, 5);
        }
        for c in &mut s.children {
            state(c, line + 7);
        }
    }
    c.span = Span::new(line, 1);
    for s in &mut c.states {
        state(s, line + 1);
    }
    c
}
