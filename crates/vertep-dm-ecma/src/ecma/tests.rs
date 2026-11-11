//! Tests that drive [`EcmaDatamodel`] through the [`Datamodel`] trait only.

use super::*;

/// A configuration given as a list of active state ids.
struct Active(&'static [&'static str]);

impl StateQuery for Active {
    fn in_state(&self, id: &str) -> bool {
        self.0.contains(&id)
    }
}

const NONE: Active = Active(&[]);

/// A configuration that can also list its active states, as the interpreter's does.
struct Listed(&'static [&'static str]);

impl StateQuery for Listed {
    fn in_state(&self, id: &str) -> bool {
        self.0.contains(&id)
    }

    fn active_states(&self) -> Option<Vec<String>> {
        Some(self.0.iter().map(|s| (*s).to_string()).collect())
    }
}

fn session() -> EcmaDatamodel {
    let mut dm = EcmaDatamodel::new();
    dm.init_session(&SessionInfo {
        sessionid: "session-1".into(),
        name: Some("chart".into()),
        ioprocessors: Value::Object(vec![(
            "http://www.w3.org/TR/scxml/#SCXMLEventProcessor".into(),
            Value::Object(vec![(
                "location".into(),
                Value::String("#_scxml_session-1".into()),
            )]),
        )]),
    })
    .unwrap();
    dm.set_environment(&Environment {
        now_ms: 1_700_000_000_000,
        seed: 42,
    });
    dm
}

fn num(n: f64) -> Value {
    Value::Number(n)
}

fn value(dm: &mut EcmaDatamodel, expr: &str) -> Value {
    dm.eval_value(expr, &NONE)
        .unwrap_or_else(|e| panic!("`{expr}` failed: {e}"))
}

fn cond(dm: &mut EcmaDatamodel, expr: &str) -> Result<bool, EvalError> {
    dm.eval_cond(expr, &NONE)
}

fn with_x(n: f64) -> EcmaDatamodel {
    let mut dm = session();
    dm.declare("x").unwrap();
    dm.assign_value("x", num(n)).unwrap();
    dm
}

// ---- Guard Purity ---------------------------------------------------------------------------

// @tg: verifies:: [[openspec:datamodel#Guard Purity#Mutating condition rejected]] {level: unit}
#[test]
fn mutating_condition_is_an_error_and_changes_nothing() {
    let mut dm = with_x(2.0);
    let before = dm.snapshot();
    let err = cond(&mut dm, "(x = 5) > 1").unwrap_err();
    assert!(err.message.contains("must not modify"), "{err}");
    assert_eq!(value(&mut dm, "x"), num(2.0));
    assert!(same_value(&before, &dm.snapshot()));
}

// @tg: verifies:: [[openspec:datamodel#Guard Purity#Read-only condition allowed]] {level: unit}
#[test]
fn read_only_condition_is_evaluated_without_changes() {
    let mut dm = with_x(2.0);
    let before = dm.snapshot();
    assert_eq!(cond(&mut dm, "x > 1"), Ok(true));
    assert_eq!(cond(&mut dm, "x > 2"), Ok(false));
    assert!(same_value(&before, &dm.snapshot()));
}

#[test]
fn conditions_cannot_mutate_through_any_path() {
    let mut dm = session();
    dm.exec_script(
        "var obj = {a: {b: 1}}; var arr = [3, 1, 2]; let lex = {n: 1}; var n = 0;\
         function bump() { n = n + 1; return true; }",
        &NONE,
    )
    .unwrap();
    let before = dm.snapshot();
    for expr in [
        "x++",
        "obj.a.b = 2",
        "delete obj.a",
        "arr.push(4)",
        "arr.sort()",
        "Object.defineProperty(obj, 'c', {value: 1})",
        "Object.setPrototypeOf(obj, null)",
        "Object.freeze(obj)",
        "fresh = 1",
        "lex.n = 2",
        "this.obj = 1",
        "Math.random() < 2 && (obj.a = 0)",
    ] {
        assert!(cond(&mut dm, expr).is_err(), "`{expr}` must be rejected");
        assert!(
            same_value(&before, &dm.snapshot()),
            "`{expr}` changed the datamodel"
        );
    }
    assert_eq!(value(&mut dm, "n"), num(0.0));
    assert_eq!(
        value(&mut dm, "typeof fresh"),
        Value::String("undefined".into())
    );
}

/// A function called from a condition runs in its own closure scope, outside the read-only
/// view; the debug-build backstop catches what it changes and rolls it back.
#[cfg(debug_assertions)]
#[test]
fn backstop_rolls_back_mutation_by_called_functions() {
    let mut dm = session();
    dm.exec_script(
        "var n = 0; var o = {k: 1}; function bump() { n = n + 1; o.k = 2; return true; }",
        &NONE,
    )
    .unwrap();
    let before = dm.snapshot();
    let err = cond(&mut dm, "bump()").unwrap_err();
    assert!(err.message.contains("rolled back"), "{err}");
    assert!(same_value(&before, &dm.snapshot()));
    assert_eq!(
        value(&mut dm, "[n, o.k]"),
        Value::Array(vec![num(0.0), num(1.0)])
    );
    assert_eq!(cond(&mut dm, "n == 0"), Ok(true));
}

#[test]
fn conditions_can_read_everything() {
    let mut dm = session();
    dm.exec_script(
        "var obj = {a: [1, 2, 3]}; var when = new Date(5); var m = new Map([['k', 1]]);\
         let lex = {n: 4}; function twice(v) { return v * 2; }",
        &NONE,
    )
    .unwrap();
    dm.set_event(&Event::named("go", EventType::Internal));
    for expr in [
        "obj.a.length == 3",
        "obj.a.indexOf(2) == 1",
        "obj.a.map(function (v) { return v + 1; })[2] == 4",
        "JSON.stringify(obj) == '{\"a\":[1,2,3]}'",
        "when.getTime() == 5",
        "m.get('k') == 1 && m.has('k') && m.size == 1",
        "lex.n == 4",
        "twice(obj.a[2]) == 6",
        "typeof undeclared === 'undefined'",
        "_event.name == 'go' && _sessionid == 'session-1'",
        "Object.keys(obj).length == 1",
        "obj === obj && obj.a === obj.a",
        "Math.max(1, 2) == 2",
    ] {
        assert_eq!(cond(&mut dm, expr), Ok(true), "`{expr}`");
    }
    assert!(cond(&mut dm, "undeclared > 1").is_err());
}

#[test]
fn random_in_a_condition_does_not_advance_the_generator() {
    let mut dm = session();
    let first = value(&mut dm, "Math.random()");
    let mut other = session();
    assert_eq!(cond(&mut other, "Math.random() < 1"), Ok(true));
    assert_eq!(value(&mut other, "Math.random()"), first);
}

#[test]
fn in_predicate_answers_from_the_configuration() {
    let mut dm = session();
    assert_eq!(dm.eval_cond("In('b')", &Active(&["a", "b"])), Ok(true));
    assert_eq!(dm.eval_cond("In(\"c\")", &Active(&["a", "b"])), Ok(false));
    dm.exec_script("function inB() { return In('b'); }", &NONE)
        .unwrap();
    assert_eq!(dm.eval_cond("inB()", &Active(&["b"])), Ok(true));
    assert_eq!(dm.eval_cond("inB()", &Active(&["a"])), Ok(false));
    assert!(dm.eval_cond("In('zz' + 'z')", &NONE).is_err());
}

#[test]
fn in_predicate_answers_computed_ids_from_listed_states() {
    let mut dm = session();
    assert_eq!(dm.eval_cond("In('s' + 1)", &Listed(&["s1"])), Ok(true));
    assert_eq!(dm.eval_cond("In('s' + 2)", &Listed(&["s1"])), Ok(false));
    dm.exec_script("var target = ['s', 2].join('');", &NONE)
        .unwrap();
    assert_eq!(dm.eval_cond("In(target)", &Listed(&["s1", "s2"])), Ok(true));
    assert_eq!(
        dm.eval_value("[In(target), In('s1')]", &Listed(&["s2"])),
        Ok(Value::Array(vec![Value::Bool(true), Value::Bool(false)]))
    );
    dm.exec_script("var seen = In(target);", &Listed(&["s2"]))
        .unwrap();
    assert_eq!(value(&mut dm, "seen"), Value::Bool(true));
    // Answers do not outlive the evaluation that supplied them.
    assert!(dm.eval_cond("In(target)", &NONE).is_err());
}

// ---- System Variables -----------------------------------------------------------------------

// @tg: verifies:: [[openspec:datamodel#System Variables#Event fields available]] {level: unit}
#[test]
fn event_fields_are_available() {
    let mut dm = session();
    dm.set_event(&Event {
        name: "go".into(),
        type_: EventType::External,
        data: Value::Object(vec![("n".into(), num(1.0))]),
        ..Event::default()
    });
    assert_eq!(value(&mut dm, "_event.data.n"), num(1.0));
    assert_eq!(value(&mut dm, "_event.name"), Value::String("go".into()));
    assert_eq!(
        value(&mut dm, "_event.type"),
        Value::String("external".into())
    );
    // Absent fields exist and are undefined (W3C test 330).
    for field in ["sendid", "origin", "origintype", "invokeid"] {
        assert_eq!(
            dm.eval_cond(
                &format!("'{field}' in _event && _event.{field} === undefined"),
                &NONE
            ),
            Ok(true),
            "{field}"
        );
    }
    assert_eq!(
        value(&mut dm, "_sessionid"),
        Value::String("session-1".into())
    );
    assert_eq!(value(&mut dm, "_name"), Value::String("chart".into()));
    assert_eq!(
        value(
            &mut dm,
            "_ioprocessors['http://www.w3.org/TR/scxml/#SCXMLEventProcessor'].location"
        ),
        Value::String("#_scxml_session-1".into())
    );
}

// @tg: verifies:: [[openspec:datamodel#System Variables#Assignment to a system variable]] {level: unit}
#[test]
fn assignment_to_a_system_variable_fails() {
    let mut dm = session();
    dm.set_event(&Event::named("go", EventType::External));
    for name in SYSTEM_VARIABLES {
        assert!(dm.assign_expr(name, "27", &NONE).is_err(), "assign {name}");
        assert!(
            dm.assign_value(name, num(27.0)).is_err(),
            "assign value {name}"
        );
        assert!(
            dm.exec_script(&format!("{name} = 27;"), &NONE).is_err(),
            "script {name}"
        );
        assert!(
            dm.exec_script(&format!("var {name} = 27;"), &NONE).is_err(),
            "var {name}"
        );
        assert!(dm.declare(name).is_err(), "declare {name}");
        assert!(!dm.is_location(name), "location {name}");
    }
    assert!(dm.assign_expr("_event.name", "'other'", &NONE).is_err());
    assert!(
        dm.exec_script("'use strict'; _event.data = 1;", &NONE)
            .is_err()
    );
    assert_eq!(
        value(&mut dm, "_sessionid"),
        Value::String("session-1".into())
    );
    assert_eq!(value(&mut dm, "_event.name"), Value::String("go".into()));
}

// ---- ECMAScript Datamodel -------------------------------------------------------------------

// @tg: verifies:: [[openspec:datamodel#ECMAScript Datamodel#Data as variables]] {level: unit}
#[test]
fn data_items_are_global_variables() {
    let mut dm = session();
    dm.declare("count").unwrap();
    assert_eq!(value(&mut dm, "count"), Value::Undefined);
    dm.assign_expr("count", "0", &NONE).unwrap();
    dm.assign_expr("count", "count + 1", &NONE).unwrap();
    assert_eq!(cond(&mut dm, "count == 1"), Ok(true));
    dm.exec_script("count = count * 10;", &NONE).unwrap();
    assert_eq!(value(&mut dm, "globalThis.count"), num(10.0));
}

// @tg: verifies:: [[openspec:datamodel#ECMAScript Datamodel#JSON content]] {level: unit}
#[test]
fn json_content_becomes_a_structured_value() {
    let mut dm = session();
    let content = dm.eval_content(r#" {"a": [1, 2]} "#).unwrap();
    assert_eq!(
        content,
        Value::Object(vec![("a".into(), Value::Array(vec![num(1.0), num(2.0)]))])
    );
    dm.declare("x").unwrap();
    dm.assign_value("x", content).unwrap();
    assert_eq!(value(&mut dm, "x.a[1]"), num(2.0));
}

#[test]
fn other_content_is_a_space_normalized_string() {
    let mut dm = session();
    assert_eq!(
        dm.eval_content("\n  this \n is\ta \n string \n"),
        Ok(Value::String("this is a string".into()))
    );
    assert_eq!(dm.eval_content("123"), Ok(num(123.0)));
    assert_eq!(dm.eval_content("<a/>"), Ok(Value::String("<a/>".into())));
}

#[test]
fn assignment_requires_an_existing_location() {
    let mut dm = session();
    // W3C test 286: assigning to an undeclared variable is an error.
    assert!(dm.assign_expr("Var1", "1", &NONE).is_err());
    assert_eq!(
        value(&mut dm, "typeof Var1"),
        Value::String("undefined".into())
    );
    assert!(dm.assign_value("Var1", num(1.0)).is_err());
    assert!(dm.assign_expr("f()", "1", &NONE).is_err());
    assert!(dm.assign_expr("a, b", "1", &NONE).is_err());
    dm.declare("o").unwrap();
    assert!(dm.assign_expr("o.p", "1", &NONE).is_err(), "o is undefined");
    dm.assign_expr("o", "{}", &NONE).unwrap();
    dm.assign_expr("o.p", "1", &NONE).unwrap();
    assert_eq!(value(&mut dm, "o.p"), num(1.0));
    assert!(dm.is_location("o.p"));
    assert!(dm.is_location("o.q"));
    assert!(!dm.is_location("o.p.q.r"));
    assert!(!dm.is_location("missing"));
    assert!(!dm.is_location("1 + 1"));
    assert!(dm.declare("not an id").is_err());
}

#[test]
fn expression_errors_are_eval_errors() {
    let mut dm = session();
    assert!(dm.eval_value("return", &NONE).is_err());
    assert!(dm.eval_value("1); globalThis.leak = (1", &NONE).is_err());
    assert_eq!(
        value(&mut dm, "typeof leak"),
        Value::String("undefined".into())
    );
    assert!(dm.eval_value("undefinedVariable", &NONE).is_err());
    assert!(
        dm.exec_script("throw new Error('boom')", &NONE)
            .unwrap_err()
            .message
            .contains("boom")
    );
    dm.set_loop_iteration_limit(10_000);
    assert!(dm.exec_script("while (true) {}", &NONE).is_err());
    assert!(
        dm.exec_script("for (var i = 0; i < 100; i++) {}", &NONE)
            .is_ok()
    );
    assert!(dm.eval_value("Symbol()", &NONE).is_err());
    assert!(
        dm.eval_value("(function () { var a = {}; a.a = a; return a; })()", &NONE)
            .is_err()
    );
}

#[test]
fn values_leave_the_engine_like_json() {
    let mut dm = session();
    assert_eq!(
        value(
            &mut dm,
            "({a: undefined, f: function () {}, d: new Date(0), n: null, s: 'x', b: 2n})"
        ),
        Value::Object(vec![
            ("a".into(), Value::Undefined),
            ("d".into(), Value::String("1970-01-01T00:00:00.000Z".into())),
            ("n".into(), Value::Null),
            ("s".into(), Value::String("x".into())),
            ("b".into(), num(2.0)),
        ])
    );
    assert_eq!(value(&mut dm, "function () {}"), Value::Undefined);
}

#[test]
fn foreach_iterates_a_shallow_copy_in_index_order() {
    let mut dm = session();
    dm.exec_script("var items = [{v: 1}, {v: 2}, 3];", &NONE)
        .unwrap();
    let values = dm.foreach_items("items", &NONE).unwrap();
    assert_eq!(values.len(), 3);
    assert_eq!(values[2], num(3.0));
    let mut sum = 0.0;
    for (index, item) in values.into_iter().enumerate() {
        dm.bind_var("item", item).unwrap();
        #[allow(clippy::cast_precision_loss)]
        dm.bind_var("index", num(index as f64)).unwrap();
        // Growing the array during iteration does not change the items visited (W3C test 525).
        dm.exec_script("items.push(0);", &NONE).unwrap();
        if let Value::Number(n) = value(&mut dm, "typeof item === 'object' ? item.v : item") {
            sum += n;
        }
        // Object items are the array's own objects, not copies.
        if index < 2 {
            assert_eq!(value(&mut dm, "item === items[index]"), Value::Bool(true));
        }
    }
    assert!((sum - 6.0).abs() < f64::EPSILON);
    assert!(dm.foreach_items("items.length", &NONE).is_err());
}

#[test]
fn script_lexical_bindings_are_visible_to_later_expressions() {
    let mut dm = session();
    assert!(dm.eval_value("y", &NONE).is_err());
    assert!(dm.assign_expr("y", "0", &NONE).is_err());
    dm.exec_script("let y = 1; const k = 2;", &NONE).unwrap();
    assert_eq!(value(&mut dm, "y + k"), num(3.0));
    dm.assign_expr("y", "y + 1", &NONE).unwrap();
    assert_eq!(cond(&mut dm, "y == 2"), Ok(true));
    assert!(
        dm.assign_expr("k", "0", &NONE).is_err(),
        "const stays constant"
    );
    assert!(dm.is_location("y"));
}

#[test]
fn bind_var_declares_missing_variables() {
    let mut dm = session();
    dm.bind_var("fresh", num(1.0)).unwrap();
    assert_eq!(value(&mut dm, "fresh"), num(1.0));
    assert!(dm.bind_var("_event", num(1.0)).is_err());
}

// ---- Deterministic Evaluation -----------------------------------------------------------------

// @tg: verifies:: [[openspec:datamodel#Deterministic Evaluation#Random numbers are reproducible]] {level: unit}
#[test]
fn random_numbers_are_reproducible() {
    let run = |seed: u64| {
        let mut dm = session();
        dm.set_environment(&Environment { now_ms: 0, seed });
        let a = value(&mut dm, "Math.random()");
        let b = value(&mut dm, "Math.random()");
        (a, b)
    };
    let (a1, b1) = run(7);
    let (a2, b2) = run(7);
    assert_eq!(a1, a2);
    assert_eq!(b1, b2);
    assert_ne!(a1, b1);
    assert_ne!(run(8).0, a1);
    let Value::Number(n) = a1 else {
        panic!("not a number")
    };
    assert!((0.0..1.0).contains(&n));
}

// @tg: verifies:: [[openspec:datamodel#Deterministic Evaluation#Time comes from the input]] {level: unit}
#[test]
fn time_comes_from_the_input() {
    let mut dm = session();
    dm.set_environment(&Environment {
        now_ms: 1_234_567_890_123,
        seed: 0,
    });
    assert_eq!(value(&mut dm, "Date.now()"), num(1_234_567_890_123.0));
    assert_eq!(
        value(&mut dm, "new Date().getTime()"),
        num(1_234_567_890_123.0)
    );
    assert_eq!(
        value(&mut dm, "new Date().getHours()"),
        num(23.0),
        "local time is UTC"
    );
    dm.set_environment(&Environment { now_ms: 5, seed: 0 });
    assert_eq!(value(&mut dm, "Date.now()"), num(5.0));
    assert_eq!(
        value(&mut dm, "typeof WeakRef"),
        Value::String("undefined".into())
    );
}

// ---- Datamodel Snapshot ---------------------------------------------------------------------

const SETUP: &str = r"
    var log = [];
    var shared = {hits: 0};
    var graph = {self: null, left: shared, right: shared, when: new Date(10), tags: new Set(['a'])};
    graph.self = graph;
    let counter = {n: 0};
    const LIMIT = 100;
    function Point(x) { this.x = x; }
    Point.prototype.double = function () { return this.x * 2; };
    var origin = new Point(21);
    class Base { constructor() { this.total = 0; } add(v) { this.total += v; return this; } }
    class Acc extends Base { add(v) { return super.add(v * 2); } static make() { return new Acc(); } }
    var acc = Acc.make();
    var Anon = class { get twice() { return 2; } };
    var obj = { method() { return 'm'; } };
    var mapped = new Map([[shared, 'shared']]);
    function handle(e) {
        counter.n += 1;
        shared.hits += e.data.n;
        acc.add(e.data.n);
        log.push(e.name + ':' + Math.floor(Math.random() * 1000) + ':' + Date.now());
        return counter.n;
    }
";

fn feed(dm: &mut EcmaDatamodel, n: u32) {
    dm.set_environment(&Environment {
        now_ms: 1000 + i64::from(n),
        seed: u64::from(n) * 31,
    });
    dm.set_event(&Event {
        name: format!("e{n}"),
        type_: EventType::External,
        data: Value::Object(vec![("n".into(), num(f64::from(n)))]),
        ..Event::default()
    });
    dm.exec_script("handle(_event);", &NONE).unwrap();
}

const PROBE: &str = "[log.join('|'), shared.hits, counter.n, LIMIT, origin.double(), acc.total, \
     acc instanceof Acc, obj.method(), graph.self === graph, graph.left === graph.right, \
     graph.when.getTime(), graph.tags.has('a'), mapped.get(shared), _event.name, Math.random(), \
     acc instanceof Base, new Anon().twice, Acc.make().add(1).total]";

// @tg: verifies:: [[openspec:datamodel#Datamodel Snapshot#Snapshot and restore]] {level: unit}
#[test]
fn restore_and_continue_equals_never_snapshotting() {
    let mut original = session();
    original.exec_script(SETUP, &NONE).unwrap();
    for n in 1..=3 {
        feed(&mut original, n);
    }
    let snapshot = original.snapshot();
    assert!(
        !matches!(&snapshot, Value::Object(f) if f.iter().any(|(k, _)| k == "error")),
        "{snapshot:?}"
    );

    let mut restored = EcmaDatamodel::new();
    restored.restore(&snapshot).unwrap();
    assert!(
        same_value(&restored.snapshot(), &snapshot),
        "restore is exact"
    );

    feed(&mut original, 4);
    feed(&mut restored, 4);
    let expected = value(&mut original, PROBE);
    assert_eq!(value(&mut restored, PROBE), expected);
    assert!(same_value(&original.snapshot(), &restored.snapshot()));
    // `const` stays constant and the system variables stay read-only after a restore.
    assert!(restored.exec_script("LIMIT = 1;", &NONE).is_err());
    assert!(restored.assign_expr("_sessionid", "1", &NONE).is_err());
    assert_eq!(
        value(&mut restored, "_sessionid"),
        Value::String("session-1".into())
    );
}

#[test]
fn snapshot_reports_values_it_cannot_restore() {
    let mut dm = session();
    dm.exec_script("var bound = function () {}.bind(null);", &NONE)
        .unwrap();
    let snapshot = dm.snapshot();
    let mut fresh = EcmaDatamodel::new();
    let err = fresh.restore(&snapshot).unwrap_err();
    assert!(err.message.contains("cannot be restored"), "{err}");
    assert!(fresh.restore(&Value::Null).is_err());
}

#[test]
fn builtins_referenced_by_data_survive_a_snapshot() {
    let mut dm = session();
    dm.exec_script("var max = Math.max; var proto = Object.prototype;", &NONE)
        .unwrap();
    let mut fresh = EcmaDatamodel::new();
    fresh.restore(&dm.snapshot()).unwrap();
    assert_eq!(
        cond(&mut fresh, "max === Math.max && proto === Object.prototype"),
        Ok(true)
    );
}
