//! One test per yaml-format scenario covered by the YAML surface.

use std::cell::RefCell;

use vertep_core::compile::CompiledChart;
use vertep_core::ir::{ExecKind, HistoryType, StateElement, StateKind, TransitionType};
use vertep_core::types::chart::{data_type, declarations, param_types, send_type};
use vertep_core::types::{Type, parse_type};
use vertep_yaml::{
    Diagnostic, ErrorKind, ExpressionChecker, ExpressionError, ReadOptions, read, read_str, write,
};

fn load(src: &str) -> vertep_core::ir::Chart {
    read_str(src).unwrap_or_else(|e| panic!("load failed:\n{e}"))
}

fn errors(src: &str) -> Vec<Diagnostic> {
    match read(src, &ReadOptions::default().with_file("chart.yaml")) {
        Ok(c) => panic!("expected errors, loaded {c:?}"),
        Err(e) => e.diagnostics,
    }
}

// ================================================================== YAML Chart Document

// @tg: verifies:: [[openspec:yaml-format#YAML Chart Document#Minimal chart]] {level: unit}
#[test]
fn minimal_chart() {
    let chart = load("vertep: 1\nname: light\nstates:\n  green: {}\n  red: {}\n");
    assert_eq!(chart.name.as_deref(), Some("light"));
    assert_eq!(chart.datamodel, "cel");
    assert!(!chart.datamodel_explicit);
    let ids: Vec<&str> = chart.states.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["green", "red"]);
    // No `initial`: the first declared state is the initial state.
    assert!(chart.initial.is_empty());
    let compiled = CompiledChart::new(chart).unwrap();
    let root_entry = compiled.default_entry(0);
    assert_eq!(compiled.ids_of(root_entry), ["green"]);
}

trait Ids {
    fn ids_of(&self, idx: &[vertep_core::compile::StateIdx]) -> Vec<String>;
}

impl Ids for CompiledChart {
    fn ids_of(&self, idx: &[vertep_core::compile::StateIdx]) -> Vec<String> {
        idx.iter().map(|&i| self.state(i).id.clone()).collect()
    }
}

// @tg: verifies:: [[openspec:yaml-format#YAML Chart Document#Missing format version]] {level: unit}
#[test]
fn missing_format_version() {
    let errs = errors("name: light\nstates:\n  green: {}\n");
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::Version);
    assert!(
        errs[0].message.contains("format version is required"),
        "{}",
        errs[0]
    );
    assert_eq!(errs[0].path, "vertep");
}

// @tg: verifies:: [[openspec:yaml-format#YAML Chart Document#Unsupported format version]] {level: unit}
#[test]
fn unsupported_format_version() {
    let errs = errors("vertep: 99\nname: light\nstates:\n  green: {}\n");
    assert_eq!(errs[0].kind, ErrorKind::Version);
    assert!(
        errs[0].message.contains("supported versions: 1"),
        "{}",
        errs[0]
    );
    assert!(errs[0].message.contains("99"));
    assert_eq!((errs[0].line, errs[0].column), (1, 9));
}

// ================================================================== States and Hierarchy

// @tg: verifies:: [[openspec:yaml-format#States and Hierarchy#Compound state with initial child]] {level: unit}
#[test]
fn compound_state_with_initial_child() {
    let chart = load(
        "vertep: 1\nname: shop\nstates:\n  checkout:\n    initial: cart\n    states:\n      \
         cart: {}\n      payment: {}\n",
    );
    let checkout = &chart.states[0];
    assert_eq!(checkout.kind(), StateKind::Compound);
    assert_eq!(checkout.initial, ["cart"]);
    assert_eq!(checkout.children[0].id, "cart");
    assert_eq!(checkout.children[1].id, "payment");
}

// @tg: verifies:: [[openspec:yaml-format#States and Hierarchy#Document order preserved]] {level: unit}
#[test]
fn document_order_preserved() {
    let chart = load(
        "vertep: 1\nname: p\nstates:\n  p:\n    type: parallel\n    states:\n      c: {}\n      \
         a: {}\n      b: {}\n  h:\n    states:\n      x: {}\n      hist: { type: history, history: deep }\n",
    );
    let p = &chart.states[0];
    assert_eq!(p.element, StateElement::Parallel);
    let ids: Vec<&str> = p.children.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["c", "a", "b"]);
    assert_eq!(
        chart.states[1].children[1].element,
        StateElement::History(HistoryType::Deep)
    );
}

// @tg: verifies:: [[openspec:yaml-format#States and Hierarchy#Duplicate state id]] {level: unit}
#[test]
fn duplicate_state_id() {
    let src = "vertep: 1\nname: d\nstates:\n  a:\n    states:\n      done: {}\n  b:\n    states:\n      done: {}\n";
    let errs = errors(src);
    let dup: Vec<&Diagnostic> = errs
        .iter()
        .filter(|e| e.kind == ErrorKind::Reference)
        .collect();
    assert_eq!(dup.len(), 2, "{errs:?}");
    assert_eq!(dup[0].line, 6);
    assert_eq!(dup[1].line, 9);
    assert!(dup[0].message.contains("also at line 9"));
    assert!(dup[1].message.contains("also at line 6"));
    assert_eq!(dup[0].path, "states.a.states.done");
}

// ================================================================== Transitions

// @tg: verifies:: [[openspec:yaml-format#Transitions#Shorthand target]] {level: unit}
#[test]
fn shorthand_target() {
    let chart =
        load("vertep: 1\nname: t\nstates:\n  idle:\n    on: { start: running }\n  running: {}\n");
    let t = &chart.states[0].transitions[0];
    assert_eq!(t.events, ["start"]);
    assert_eq!(t.targets, ["running"]);
    assert_eq!(t.cond, None);
}

// @tg: verifies:: [[openspec:yaml-format#Transitions#Guarded alternatives in order]] {level: unit}
#[test]
fn guarded_alternatives_in_order() {
    let chart = load(
        "vertep: 1\nname: t\nstates:\n  s:\n    on: { pay: [ { cond: \"ev.amount > 100\", target: review }, \
         { target: paid } ] }\n  review: {}\n  paid: {}\n",
    );
    let ts = &chart.states[0].transitions;
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].events, ["pay"]);
    assert_eq!(ts[0].cond.as_deref(), Some("ev.amount > 100"));
    assert_eq!(ts[0].targets, ["review"]);
    assert_eq!(ts[1].cond, None);
    assert_eq!(ts[1].targets, ["paid"]);
}

// @tg: verifies:: [[openspec:yaml-format#Transitions#Eventless transition]] {level: unit}
#[test]
fn eventless_transition() {
    let chart = load(
        "vertep: 1\nname: t\ncontext: { ready: { init: \"false\" } }\nstates:\n  s:\n    \
         always: [ { cond: \"ctx.ready\", target: next, type: internal } ]\n  next: {}\n",
    );
    let t = &chart.states[0].transitions[0];
    assert!(t.events.is_empty());
    assert_eq!(t.cond.as_deref(), Some("ctx.ready"));
    assert_eq!(t.targets, ["next"]);
    assert_eq!(t.kind, TransitionType::Internal);
    assert!(t.kind_explicit);
}

// ================================================================== Executable Content

// @tg: verifies:: [[openspec:yaml-format#Executable Content#Assign and send]] {level: unit}
#[test]
fn assign_and_send() {
    let chart = load(
        "vertep: 1\nname: e\ncontext: { total: { init: \"0\" } }\nstates:\n  s:\n    entry:\n      \
         - { assign: { location: ctx.total, expr: \"ctx.total + ev.amount\" } }\n      \
         - { send: { event: receipt, target: \"#_parent\" } }\n",
    );
    let entry = &chart.states[0].onentry;
    assert_eq!(entry.len(), 1);
    match &entry[0][0].kind {
        ExecKind::Assign { location, expr, .. } => {
            assert_eq!(location, "ctx.total");
            assert_eq!(expr.as_deref(), Some("ctx.total + ev.amount"));
        }
        other => panic!("expected assign, got {other:?}"),
    }
    match &entry[0][1].kind {
        ExecKind::Send(s) => {
            assert_eq!(s.event.as_deref(), Some("receipt"));
            assert_eq!(s.target.as_deref(), Some("#_parent"));
        }
        other => panic!("expected send, got {other:?}"),
    }
}

// @tg: verifies:: [[openspec:yaml-format#Executable Content#Two keys in one item]] {level: unit}
#[test]
fn two_keys_in_one_item() {
    let errs = errors(
        "vertep: 1\nname: e\nstates:\n  s:\n    entry:\n      - raise: go\n        log: { expr: \"1\" }\n",
    );
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].line, 6);
    assert_eq!(errs[0].path, "states.s.entry[0]");
    assert!(
        errs[0].message.contains("exactly one key is allowed"),
        "{}",
        errs[0]
    );
    assert!(errs[0].message.contains("`raise` and `log`"));
}

// ================================================================== Invoke and Data

// @tg: verifies:: [[openspec:yaml-format#Invoke and Data in YAML#Invoke with params]] {level: unit}
#[test]
fn invoke_with_params() {
    let chart = load(
        "vertep: 1\nname: i\ncontext: { url: { init: \"'x'\" } }\nstates:\n  fetching:\n    \
         invoke: { id: loader, type: \"vt:actor\", src: \"vt:actor/Loader\", params: { url: \"ctx.url\" } }\n",
    );
    let inv = &chart.states[0].invokes[0];
    assert_eq!(inv.id.as_deref(), Some("loader"));
    assert_eq!(inv.type_.as_deref(), Some("vt:actor"));
    assert_eq!(inv.src.as_deref(), Some("vt:actor/Loader"));
    assert_eq!(inv.params.len(), 1);
    assert_eq!(inv.params[0].name, "url");
    assert_eq!(inv.params[0].expr.as_deref(), Some("ctx.url"));
}

// @tg: verifies:: [[openspec:yaml-format#Invoke and Data in YAML#Typed context field]] {level: unit}
#[test]
fn typed_context_field() {
    let chart = load(
        "vertep: 1\nname: c\ncontext: { total: { type: decimal, init: \"0\" } }\nstates:\n  s: {}\n",
    );
    let d = &chart.data[0];
    assert_eq!(d.id, "total");
    assert_eq!(d.expr.as_deref(), Some("0"));
    assert_eq!(data_type(d), Some(Ok(Type::DECIMAL)));
    let types = vertep_core::types::check_chart(
        &chart,
        &vertep_core::types::BasicTyper::for_datamodel("cel"),
    )
    .unwrap();
    assert_eq!(
        types.context_type(),
        Type::record([("total", Type::DECIMAL)])
    );
}

// ================================================================== Types block

// @tg: verifies:: [[openspec:yaml-format#YAML Types Block#Record and union types]] {level: unit}
#[test]
fn record_and_union_types() {
    let chart = load(
        "vertep: 1\nname: t\ntypes:\n  Order: { id: string, total: decimal }\n  \
         Payment: { oneOf: { Card: { last4: string }, Cash: null } }\n  \
         Lines: \"list<optional<Order>>\"\nstates:\n  s: {}\n",
    );
    let d = declarations(&chart).unwrap().unwrap();
    assert_eq!(
        d.lookup("Order"),
        Some(&parse_type("{ id: string, total: decimal }").unwrap())
    );
    assert_eq!(
        d.lookup("Payment"),
        Some(&parse_type("Card { last4: string } | Cash").unwrap())
    );
    assert_eq!(
        d.lookup("Lines"),
        Some(&Type::list(Type::optional(Type::named("Order"))))
    );
}

// @tg: verifies:: [[openspec:yaml-format#YAML Types Block#Event payload type]] {level: unit}
#[test]
fn event_payload_type() {
    let src = "vertep: 1\nname: t\ntypes:\n  Payment: { oneOf: { Card: { last4: string }, Cash: null } }\n\
               events: { pay: Payment, cancel: null }\nstates:\n  s:\n    entry:\n      \
               - send: { event: pay, params: { last4: \"'1234'\" } }\n";
    let chart = load(
        src.replace(
            "params: { last4: \"'1234'\" }",
            "content: { expr: \"ctx\" }",
        )
        .as_str(),
    );
    let d = declarations(&chart).unwrap().unwrap();
    assert_eq!(
        d.event("pay").unwrap().payload,
        Some(Type::named("Payment"))
    );
    assert_eq!(d.event("cancel").unwrap().payload, None);
    // The type checker checks sends of `pay` against `Payment`: a union takes no params.
    let errs = errors(src);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!(errs[0].kind, ErrorKind::Type);
    assert!(errs[0].message.contains("is not a record"), "{}", errs[0]);
    assert_eq!(errs[0].path, "states.s.entry[0]");
    assert_eq!(errs[0].line, 9);
}

#[test]
fn send_payload_and_param_types_are_spelled_in_yaml() {
    let chart = load(
        "vertep: 1\nname: t\nstates:\n  s:\n    entry:\n      - send:\n          event: pay\n          \
         payload_type: { amount: decimal }\n          params:\n            - { name: amount, expr: \"1.5\", type: decimal }\n",
    );
    let exec = &chart.states[0].onentry[0][0];
    assert_eq!(
        send_type(exec),
        Some(Ok(Type::record([("amount", Type::DECIMAL)])))
    );
    let fields = param_types(&exec.extensions).unwrap().unwrap();
    assert_eq!(fields[0].ty, Type::DECIMAL);
    let back = load(&write(&chart).unwrap());
    assert_eq!(back, chart);
}

// ================================================================== Round trip

// @tg: verifies:: [[openspec:yaml-format#Lossless YAML Round Trip#Stable output]] {level: unit}
#[test]
fn stable_output() {
    let chart = load(include_str!("data/order.yaml"));
    let a = write(&chart).unwrap();
    let b = write(&chart).unwrap();
    assert_eq!(a, b);
    let again = write(&load(&a)).unwrap();
    assert_eq!(a, again, "writing the re-read chart gives the same bytes");
}

// ================================================================== Source-located errors

// @tg: verifies:: [[openspec:yaml-format#Source-Located Errors#Unknown target located]] {level: unit}
#[test]
fn unknown_target_located() {
    let src = include_str!("data/order.yaml").replace("target: payment", "target: paymnt");
    let line = src.lines().position(|l| l.contains("paymnt")).unwrap() + 1;
    assert_eq!(line, 14, "the fixture puts the target on line 14");
    let errs = errors(&src);
    assert_eq!(errs.len(), 1, "{errs:?}");
    let e = &errs[0];
    assert_eq!(e.kind, ErrorKind::Reference);
    assert_eq!(e.line, 14);
    assert_eq!(e.file.as_deref(), Some("chart.yaml"));
    assert_eq!(e.path, "states.checkout.states.cart.on.pay[1].target");
    assert!(e.message.contains("`paymnt`"), "{e}");
    assert!(e.message.contains("did you mean `payment`?"), "{e}");
    assert!(e.to_string().starts_with("chart.yaml:14:"), "{e}");
}

// @tg: verifies:: [[openspec:yaml-format#Source-Located Errors#YAML syntax error]] {level: unit}
#[test]
fn yaml_syntax_error() {
    let src = "vertep: 1\nname: x\nstates:\n  a:\n    on: { go: b }\n   bad: indentation\n";
    let errs = errors(src);
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::Syntax);
    assert_eq!(errs[0].line, 6);
    assert!(errs[0].column > 0);
}

#[test]
fn every_error_is_reported_together() {
    let src = "vertep: 1\nname: x\nstates:\n  a:\n    on: { go: nowhere }\n    entry: [ { raize: x } ]\n  \
               b: { tipe: final }\n";
    let errs = errors(src);
    let paths: Vec<&str> = errs.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "states.a.on.go.target",
            "states.a.entry[0].raize",
            "states.b.tipe"
        ],
        "{errs:?}"
    );
    assert!(errs[1].message.contains("did you mean `raise`?"));
    assert!(errs[2].message.contains("did you mean `type`?"));
}

// ================================================================== Expression strings

/// A stand-in expression parser: rejects a trailing binary operator, records datamodels.
struct TrailingOperator {
    seen: RefCell<Vec<String>>,
}

impl ExpressionChecker for TrailingOperator {
    fn check(&self, datamodel: &str, expr: &str) -> Result<(), ExpressionError> {
        self.seen.borrow_mut().push(datamodel.to_string());
        let t = expr.trim_end();
        match t.chars().last() {
            Some('>' | '<' | '+' | '-' | '*' | '/') => Err(ExpressionError {
                offset: t.len(),
                message: "unexpected end of expression, expected an operand".into(),
            }),
            _ => Ok(()),
        }
    }
}

// @tg: verifies:: [[openspec:yaml-format#YAML Uses Expression Strings of Its Datamodel#CEL expression parse error]] {level: unit}
#[test]
fn cel_expression_parse_error() {
    let checker = TrailingOperator {
        seen: RefCell::new(Vec::new()),
    };
    let src = "vertep: 1\nname: x\ncontext: { total: { init: \"0\" } }\nstates:\n  a:\n    \
               always: [ { cond: \"ctx.total >\", target: b } ]\n  b: {}\n";
    let err = read(src, &ReadOptions::default().with_expressions(&checker)).unwrap_err();
    assert_eq!(err.diagnostics.len(), 1);
    let d = &err.diagnostics[0];
    assert_eq!(d.kind, ErrorKind::Expression);
    assert_eq!(d.path, "states.a.always[0].cond");
    // `cond: "ctx.total >"` starts at column 23; the error is at the end of the string.
    let line = src.lines().nth(5).unwrap();
    let quote = line.find("\"ctx").unwrap() + 1;
    assert_eq!(d.line, 6);
    assert_eq!(d.column as usize, quote + 1 + "ctx.total >".len());
    assert!(d.message.starts_with("cel datamodel:"), "{d}");
    assert!(checker.seen.borrow().iter().all(|m| m == "cel"));
}

// @tg: verifies:: [[openspec:yaml-format#YAML Uses Expression Strings of Its Datamodel#Different datamodel declared]] {level: unit}
#[test]
fn different_datamodel_declared() {
    let checker = TrailingOperator {
        seen: RefCell::new(Vec::new()),
    };
    let src = "vertep: 1\nname: x\ndatamodel: ecmascript\ncontext: { n: { init: \"0\" } }\nstates:\n  a:\n    \
               always: [ { cond: \"n > 1\", target: b } ]\n  b: {}\n";
    let chart = read(src, &ReadOptions::default().with_expressions(&checker)).unwrap();
    assert_eq!(chart.datamodel, "ecmascript");
    assert!(chart.datamodel_explicit);
    let seen = checker.seen.borrow();
    assert_eq!(seen.len(), 2);
    assert!(seen.iter().all(|m| m == "ecmascript"), "{seen:?}");
}

#[test]
fn typed_charts_are_written_with_shorthands() {
    let src = "vertep: 1\nname: t\ntypes:\n  Pay: { amount: decimal }\nevents: { pay: Pay, go: null }\n\
               context:\n  total: { type: decimal, init: \"0\" }\nstates:\n  s:\n    entry:\n      \
               - send:\n          event: pay\n          payload_type: Pay\n          params:\n            \
               - { name: amount, expr: \"1.5\", type: decimal }\n      - call: record_total\n    \
               donedata: { params: [ { name: total, expr: ctx.total, type: decimal } ] }\n    type: final\n";
    let chart = load(src);
    let out = write(&chart).unwrap();
    for key in [
        "types:",
        "events:",
        "type: decimal",
        "payload_type: Pay",
        "call: record_total",
    ] {
        assert!(out.contains(key), "missing `{key}` in:\n{out}");
    }
    assert!(
        !out.contains("extensions:"),
        "no raw extensions needed:\n{out}"
    );
    assert_eq!(load(&out), chart);
}
