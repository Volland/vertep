//! Unit tests of the type language, the value checker and the chart type checker.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::compile::CompiledChart;
use crate::datamodel::{Environment, Event, EventType, SessionInfo, Value};
use crate::ir::{Chart, Data, Exec, ExecKind, Param, Send, Span, State, StateElement, Transition};
use crate::null::NullDatamodel;
use crate::session::{self, Input, StartInput};
use crate::types::chart::{
    data_type, declarations, param_types, parse_types_element, send_type, set_data_type,
    set_declarations, set_param_type, set_send_type, types_element_xml,
};
use crate::types::check::is_assignable;
use crate::types::decl::DeclErrorKind;
use crate::types::events::{descriptor_matches, error_payload_type};
use crate::types::*;

fn s(x: &str) -> Value {
    Value::String(x.to_string())
}

fn obj(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn kinds(r: Result<(), Vec<TypeError>>) -> Vec<TypeErrorKind> {
    r.err()
        .unwrap_or_default()
        .into_iter()
        .map(|e| e.kind)
        .collect()
}

fn decls(src: &str) -> Declarations {
    parse_declarations(src).expect("declarations parse")
}

// ============================================================================ primitives

// @tg: verifies:: [[openspec:type-system#Primitive Types#Integer outside 64-bit range is rejected]] {level: unit}
#[test]
fn integer_outside_64_bit_range_is_rejected() {
    let ty = Type::record([("n", Type::INT)]);
    let v = obj(&[("n", Value::Number(9_223_372_036_854_775_808.0))]);
    let errs = check_value(&ty, &v).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, TypeErrorKind::IntOutOfRange);
    assert_eq!(errs[0].path.to_string(), "$.n");
    let msg = errs[0].to_string();
    assert!(msg.contains("expected int"), "{msg}");
    assert!(msg.contains("$.n"), "{msg}");
    // In range and integral is fine; a fraction is not an int.
    assert!(check_value(&Type::INT, &Value::Number(-9_223_372_036_854_775_808.0)).is_ok());
    assert!(check_value(&Type::INT, &Value::Number(42.0)).is_ok());
    assert_eq!(
        kinds(check_value(&Type::INT, &Value::Number(1.5))),
        vec![TypeErrorKind::IntOutOfRange]
    );
}

// @tg: verifies:: [[openspec:type-system#Primitive Types#Datetime without offset is rejected]] {level: unit}
#[test]
fn datetime_without_offset_is_rejected() {
    let errs = check_value(&Type::DATETIME, &s("2026-11-11T10:00:00")).unwrap_err();
    assert_eq!(errs[0].kind, TypeErrorKind::DatetimeOffsetRequired);
    assert!(errs[0].to_string().contains("offset"));
    assert!(errs[0].to_string().contains("required"));
    assert!(check_value(&Type::DATETIME, &s("2026-11-11T10:00:00Z")).is_ok());
    assert!(check_value(&Type::DATETIME, &s("2026-11-11T10:00:00.25+02:00")).is_ok());
    assert!(check_value(&Type::DATETIME, &s("2026-02-30T10:00:00Z")).is_err());
}

// @tg: verifies:: [[openspec:type-system#Primitive Types#Relative IRI is rejected]] {level: unit}
#[test]
fn relative_iri_is_rejected() {
    let errs = check_value(&Type::IRI, &s("orders/42")).unwrap_err();
    assert_eq!(errs[0].kind, TypeErrorKind::RelativeIri);
    assert!(errs[0].to_string().contains("not absolute"));
    assert!(check_value(&Type::IRI, &s("https://example.org/orders/42")).is_ok());
    assert!(check_value(&Type::IRI, &s("urn:uuid:1234")).is_ok());
    assert!(check_value(&Type::IRI, &s("https://w3id.org/vertep/vt#Chart")).is_ok());
    assert_eq!(
        kinds(check_value(&Type::IRI, &s("https://ex.org/a b"))),
        vec![TypeErrorKind::InvalidIri]
    );
}

#[test]
fn primitives_reject_wrong_kinds_and_decimals_are_exact() {
    assert!(check_value(&Type::BOOL, &Value::Bool(true)).is_ok());
    assert!(check_value(&Type::BOOL, &s("true")).is_err());
    assert!(check_value(&Type::DECIMAL, &s("12.50")).is_ok());
    assert!(check_value(&Type::DECIMAL, &Value::Number(3.0)).is_ok());
    assert_eq!(
        kinds(check_value(&Type::DECIMAL, &s("12,50"))),
        vec![TypeErrorKind::InvalidDecimal]
    );
    assert_eq!(encode(&Type::DECIMAL, &Value::Number(2.5)), Ok(s("2.5")));
    assert_eq!(encode(&Type::DECIMAL, &s("0.10")), Ok(s("0.10")));
}

// ============================================================================ composites

fn order() -> Type {
    Type::record([("id", Type::STRING), ("amount", Type::DECIMAL)])
}

// @tg: verifies:: [[openspec:type-system#Composite Types#Record with a missing required field]] {level: unit}
#[test]
fn record_with_missing_required_field() {
    let errs = check_value(&order(), &obj(&[("id", s("o-1"))])).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, TypeErrorKind::MissingField("amount".into()));
    assert!(errs[0].to_string().contains("`amount`"));
}

// @tg: verifies:: [[openspec:type-system#Composite Types#Record with an unknown field]] {level: unit}
#[test]
fn record_with_unknown_field() {
    let v = obj(&[("id", s("o-1")), ("amount", s("3")), ("note", s("x"))]);
    let errs = check_value(&order(), &v).unwrap_err();
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, TypeErrorKind::UnknownField("note".into()));
    assert!(errs[0].to_string().contains("`note`"));
}

// @tg: verifies:: [[openspec:type-system#Composite Types#Tagged union selects its variant by tag]] {level: unit}
#[test]
fn tagged_union_selects_its_variant_by_tag() {
    let payment = parse_type("Card { last4: string } | Cash").unwrap();
    assert!(check_value(&payment, &obj(&[("tag", s("Cash"))])).is_ok());
    let card = obj(&[("tag", s("Card")), ("value", obj(&[("last4", s("4242"))]))]);
    assert!(check_value(&payment, &card).is_ok());
    let errs = check_value(&payment, &obj(&[("tag", s("Card"))])).unwrap_err();
    assert_eq!(errs[0].kind, TypeErrorKind::MissingPayload("Card".into()));
    assert!(errs[0].to_string().contains("`Card`"));
    // Wrong payload is located under the variant.
    let bad = obj(&[
        ("tag", s("Card")),
        ("value", obj(&[("last4", Value::Number(1.0))])),
    ]);
    let errs = check_value(&payment, &bad).unwrap_err();
    assert_eq!(errs[0].path.to_string(), "$<Card>.last4");
    assert_eq!(
        kinds(check_value(&payment, &obj(&[("tag", s("Cheque"))]))),
        vec![TypeErrorKind::UnknownTag("Cheque".into())]
    );
}

// @tg: verifies:: [[openspec:type-system#Composite Types#Optional accepts absence]] {level: unit}
#[test]
fn optional_accepts_absence() {
    let ty = Type::record([("id", Type::STRING), ("note", Type::optional(Type::STRING))]);
    let v = obj(&[("id", s("o-1"))]);
    assert!(check_value(&ty, &v).is_ok());
    // The canonical encoding keeps the field absent rather than inventing an empty string.
    let enc = encode(&ty, &v).unwrap();
    assert_eq!(enc, obj(&[("id", s("o-1"))]));
    let Value::Object(fields) = enc else {
        panic!("record encodes to an object")
    };
    assert!(fields.iter().all(|(k, _)| k != "note"));
    // Null also reads as absent.
    assert!(check_value(&ty, &obj(&[("id", s("o-1")), ("note", Value::Null)])).is_ok());
}

#[test]
fn constructors_nest_and_report_every_error_with_paths() {
    let ty = Type::list(Type::record([
        ("items", Type::list(Type::optional(Type::INT))),
        ("at", Type::DATETIME),
    ]));
    let v = Value::Array(vec![
        obj(&[("items", Value::Array(vec![Value::Number(1.0), Value::Null]))]),
        obj(&[
            ("items", Value::Array(vec![s("x")])),
            ("at", s("2026-11-11T10:00:00")),
        ]),
    ]);
    let errs = check_value(&ty, &v).unwrap_err();
    let shown: Vec<String> = errs.iter().map(|e| e.path.to_string()).collect();
    assert_eq!(shown, vec!["$[0]", "$[1].items[0]", "$[1].at"]);
    assert_eq!(errs[0].kind, TypeErrorKind::MissingField("at".into()));
}

#[test]
fn canonical_encoding_sorts_fields_and_normalizes_datetimes() {
    let ty = Type::record([("z", Type::DATETIME), ("a", Type::DECIMAL)]);
    let v = obj(&[
        ("z", s("2026-11-11T01:30:00+02:00")),
        ("a", Value::Number(1.0)),
    ]);
    assert_eq!(
        encode(&ty, &v).unwrap(),
        obj(&[("a", s("1")), ("z", s("2026-11-10T23:30:00Z"))])
    );
}

// ============================================================================ recursion

// @tg: verifies:: [[openspec:type-system#Recursive and Cyclic Type Definitions#Recursive tree through a list]] {level: unit}
#[test]
fn recursive_tree_through_a_list() {
    let d = decls("type Node = { label: string, children: list<Node> };");
    assert_eq!(d.validate(), vec![]);
    let leaf = obj(&[("label", s("b")), ("children", Value::Array(vec![]))]);
    let tree = obj(&[("label", s("a")), ("children", Value::Array(vec![leaf]))]);
    assert!(d.check(&Type::named("Node"), &tree).is_ok());
    let bad = obj(&[
        ("label", s("a")),
        ("children", Value::Array(vec![obj(&[("label", s("b"))])])),
    ]);
    let errs = d.check(&Type::named("Node"), &bad).unwrap_err();
    assert_eq!(errs[0].path.to_string(), "$.children[0]");
}

// @tg: verifies:: [[openspec:type-system#Recursive and Cyclic Type Definitions#Unguarded self reference]] {level: unit}
#[test]
fn unguarded_self_reference() {
    let d = decls("type Loop = { next: Loop };");
    let errs = d.validate();
    assert_eq!(errs.len(), 1);
    assert_eq!(
        errs[0].kind,
        DeclErrorKind::UnguardedCycle(vec!["Loop".into(), "Loop".into()])
    );
    assert!(errs[0].to_string().contains("Loop -> Loop"));
    // Mutual recursion is reported once; guarded recursion through optional is fine.
    let d = decls("type A = { b: B }; type B = { a: A }; type C = { c: optional<C> };");
    let errs = d.validate();
    assert_eq!(errs.len(), 1);
    assert!(errs[0].to_string().contains("A -> B -> A"));
}

#[test]
fn declarations_report_unknown_names_duplicates_and_reserved_names() {
    let d = decls(
        "type Order = { id: string, id: int }; type Order = int; type P = Ordr | Cash Cash; \
         type U = Card | Card;",
    );
    let errs: Vec<String> = d.validate().iter().map(|e| e.to_string()).collect();
    assert!(
        errs.iter()
            .any(|e| e.contains("field `id` is declared more than once"))
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("type `Order` is declared more than once"))
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("variant `Card` is declared more than once"))
    );
    assert!(errs.iter().any(|e| e.contains("unknown type `Cash`")));
    assert!(parse_declarations("type int = string;").is_err());
}

#[test]
fn unknown_type_names_get_suggestions() {
    let d = decls("type Order = { id: string }; type Payment = Card | Cash;");
    let errs = d.validate_type(&Type::named("Ordr"), "data \"x\"", Span::NONE);
    assert_eq!(
        errs[0].kind,
        DeclErrorKind::UnknownType {
            name: "Ordr".into(),
            suggestion: Some("Order".into())
        }
    );
    assert!(errs[0].to_string().contains("did you mean `Order`?"));
    let errs = d.validate_type(&Type::named("Zzzzzz"), "x", Span::NONE);
    assert!(!errs[0].to_string().contains("did you mean"));
}

// ============================================================================ syntax

#[test]
fn type_syntax_prints_and_parses_back() {
    for src in [
        "bool",
        "list<optional<int>>",
        "{}",
        "{ id: string, total: decimal }",
        "Card { last4: string } | Cash",
        "| Only",
        "A (B | C) | D list<int>",
        "{ \"first-name\": string, list: value }",
        "\"int\" | \"string\"",
        "Order",
    ] {
        let t = parse_type(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert_eq!(t.to_string(), src, "canonical print of {src}");
        assert_eq!(parse_type(&t.to_string()), Ok(t));
    }
    assert_eq!(
        parse_type("Card {last4:string}|Cash").unwrap(),
        Type::union([
            ("Card", Some(Type::record([("last4", Type::STRING)]))),
            ("Cash", None)
        ])
    );
    let e = parse_type("list<int").unwrap_err();
    assert_eq!(e.offset, 8);
    assert!(parse_type("int | string").is_err());
    assert!(parse_type("Order<int>").is_err());
}

#[test]
fn declaration_blocks_round_trip_through_the_types_element() {
    let d = decls(
        "# order types\n type Pay = { amount: decimal };\n type Approval = { by: iri };\n\
         event start; event pay: Pay; event order.created: list<Pay>; event \"odd name\": int",
    );
    assert_eq!(d.types.len(), 2);
    assert_eq!(d.events.len(), 4);
    assert_eq!(d.events[2].name, "order.created");
    let text = d.to_text();
    assert_eq!(parse_declarations(&text), Ok(d.clone()));
    let xml = types_element_xml(&d);
    assert!(xml.contains("list&lt;Pay&gt;"));
    assert_eq!(parse_types_element(&xml), Ok(d.clone()));
    let foreign = "<v:types xmlns:v=\"https://w3id.org/vertep/vt#\"><![CDATA[event a: list<int>;]]></v:types>";
    assert_eq!(
        parse_types_element(foreign).unwrap().events[0].payload,
        Some(Type::list(Type::INT))
    );
    assert_eq!(
        parse_types_element("<vt:types/>"),
        Ok(Declarations::default())
    );
}

// ============================================================================ IR carriers

#[test]
fn annotations_ride_on_extension_nodes() {
    let mut chart = Chart::default();
    let d = decls("type Pay = { amount: decimal }; event pay: Pay;");
    set_declarations(&mut chart, Some(&d));
    assert_eq!(declarations(&chart), Some(Ok(d.clone())));
    set_declarations(&mut chart, Some(&d));
    assert_eq!(chart.extensions.len(), 1, "setting twice replaces");

    let mut data = data_item("total", Some("0"));
    set_data_type(&mut data, Some(&Type::DECIMAL));
    assert_eq!(data_type(&data), Some(Ok(Type::DECIMAL)));
    set_data_type(&mut data, None);
    assert_eq!(data_type(&data), None);

    let mut exec = Exec::new(ExecKind::Send(Box::default()));
    set_send_type(&mut exec, Some(&Type::named("Pay")));
    set_param_type(&mut exec.extensions, "amount", Some(&Type::DECIMAL));
    set_param_type(
        &mut exec.extensions,
        "note",
        Some(&Type::optional(Type::STRING)),
    );
    assert_eq!(send_type(&exec), Some(Ok(Type::named("Pay"))));
    let fields = param_types(&exec.extensions).unwrap().unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[1].ty, Type::optional(Type::STRING));
    // Attributes stay sorted by local name: paramTypes before type.
    assert_eq!(exec.extensions[0].local, "paramTypes");
    assert_eq!(exec.extensions[1].local, "type");
    set_param_type(&mut exec.extensions, "amount", None);
    set_param_type(&mut exec.extensions, "note", None);
    assert_eq!(param_types(&exec.extensions), None);
}

// ============================================================================ chart checking

fn data_item(id: &str, expr: Option<&str>) -> Data {
    Data {
        id: id.to_string(),
        src: None,
        expr: expr.map(str::to_string),
        content: None,
        extensions: Vec::new(),
        span: Span::NONE,
    }
}

fn typed_data(id: &str, ty: &str, expr: Option<&str>) -> Data {
    let mut d = data_item(id, expr);
    set_data_type(&mut d, Some(&parse_type(ty).unwrap()));
    d
}

fn send(event: &str, params: &[(&str, &str)], line: u32) -> Exec {
    let mut e = Exec::new(ExecKind::Send(Box::new(Send {
        event: Some(event.to_string()),
        params: params
            .iter()
            .map(|(n, x)| Param {
                name: n.to_string(),
                expr: Some(x.to_string()),
                location: None,
                span: Span::new(line, 9),
            })
            .collect(),
        ..Send::default()
    })));
    e.span = Span::new(line, 5);
    e
}

fn transition(events: &str, cond: Option<&str>, target: &str, body: Vec<Exec>) -> Transition {
    Transition {
        events: events.split_whitespace().map(str::to_string).collect(),
        cond: cond.map(str::to_string),
        targets: vec![target.to_string()],
        body,
        span: Span::new(7, 3),
        ..Transition::default()
    }
}

/// A two-state CEL chart: `idle` with the given transitions, then final `done`.
fn chart(decl: &str, data: Vec<Data>, transitions: Vec<Transition>) -> Chart {
    let mut idle = State::new("idle", StateElement::State);
    idle.transitions = transitions;
    let done = State::new("done", StateElement::Final);
    let mut c = Chart {
        name: Some("order".into()),
        datamodel: "cel".into(),
        datamodel_explicit: true,
        data,
        states: vec![idle, done],
        ..Chart::default()
    };
    if !decl.is_empty() {
        set_declarations(&mut c, Some(&decls(decl)));
    }
    c
}

fn cel() -> BasicTyper {
    BasicTyper::for_datamodel("cel")
}

// @tg: verifies:: [[openspec:type-system#Chart Type Checking#Send with a payload of the wrong shape]] {level: unit}
#[test]
fn send_with_a_payload_of_the_wrong_shape() {
    let c = chart(
        "event pay: { amount: decimal };",
        vec![],
        vec![transition(
            "go",
            None,
            "done",
            vec![send("pay", &[("amount", "'ten'")], 12)],
        )],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert_eq!(diags.len(), 1, "{diags:?}");
    let d = &diags[0];
    assert_eq!(d.kind, DiagnosticKind::Send);
    assert_eq!(d.span.line, 12, "located at the <send>");
    assert!(d.location.contains("send \"pay\""), "{}", d.location);
    assert!(d.message.contains("`amount`"), "{}", d.message);
    assert!(d.message.contains("string"), "{}", d.message);
    // A decimal literal is fine.
    let ok = chart(
        "event pay: { amount: decimal };",
        vec![],
        vec![transition(
            "go",
            None,
            "done",
            vec![send("pay", &[("amount", "10.5")], 12)],
        )],
    );
    assert!(check_chart(&ok, &cel()).is_ok());
}

// @tg: verifies:: [[openspec:type-system#Chart Type Checking#Guard reads an undeclared context field]] {level: unit}
#[test]
fn guard_reads_an_undeclared_context_field() {
    let c = chart(
        "",
        vec![typed_data("total", "decimal", Some("0"))],
        vec![transition(
            "authorized",
            Some("ctx.total > ctx.limit"),
            "done",
            vec![],
        )],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].kind, DiagnosticKind::Guard);
    assert!(diags[0].message.contains("`limit`"));
    assert!(diags[0].location.contains("transition 0 (on authorized)"));
    assert_eq!(diags[0].offset, Some(16), "offset of `limit` in the guard");
}

// @tg: verifies:: [[openspec:type-system#Chart Type Checking#Several errors are reported together]] {level: unit}
#[test]
fn several_errors_are_reported_together() {
    let c = chart(
        "event pay: { amount: decimal };",
        vec![typed_data("count", "int", Some("'zero'"))],
        vec![
            transition("go", Some("ctx.missing"), "done", vec![]),
            transition(
                "go",
                None,
                "done",
                vec![send("pay", &[("amount", "true")], 20)],
            ),
        ],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert_eq!(diags.len(), 3, "{diags:?}");
    let kinds: Vec<DiagnosticKind> = diags.iter().map(|d| d.kind).collect();
    assert_eq!(
        kinds,
        vec![
            DiagnosticKind::Data,
            DiagnosticKind::Guard,
            DiagnosticKind::Send
        ]
    );
    assert!(diags.iter().all(|d| !d.location.is_empty()));
}

// @tg: verifies:: [[openspec:type-system#Chart Type Checking#Instantiation is refused after type errors]] {level: unit}
#[test]
fn instantiation_is_refused_after_type_errors() {
    let bad = chart(
        "",
        vec![typed_data("n", "int", Some("'x'"))],
        vec![transition("go", None, "done", vec![])],
    );
    match compile_checked(bad, &cel()) {
        Err(InstantiateError::Type(diags)) => assert_eq!(diags.len(), 1),
        other => panic!("expected type errors, got {other:?}"),
    }
    let good = chart(
        "",
        vec![typed_data("n", "int", Some("1"))],
        vec![transition("go", None, "done", vec![])],
    );
    let (compiled, types) = compile_checked(good, &cel()).expect("well-typed chart compiles");
    assert!(types.typed);
    assert_eq!(compiled.chart().states.len(), 2);
}

// @tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Untyped SCXML chart runs unchanged]] {level: unit}
#[test]
fn untyped_chart_runs_unchanged() {
    // An untyped chart as the SCXML reader produces it (null datamodel, no vt: annotations).
    let mut a = State::new("a", StateElement::State);
    a.transitions = vec![transition("go", Some("In('a')"), "b", vec![])];
    let mut c = Chart {
        states: vec![a, State::new("b", StateElement::Final)],
        ..Chart::default()
    };
    c.data.push(data_item("x", None));
    assert!(!is_typed(&c));
    let types = check_chart(&c, &BasicTyper::for_datamodel("null")).expect("no type errors");
    assert!(!types.typed);
    assert_eq!(types.context, vec![Field::new("x", Type::Value)]);

    let plain = CompiledChart::new(c.clone()).unwrap();
    let (gated, _) = compile_checked(c, &BasicTyper::default()).unwrap();
    assert_eq!(plain.chart(), gated.chart());
    let run = |chart: &CompiledChart| {
        let mut dm = NullDatamodel::new();
        let start = session::start(
            chart,
            StartInput {
                info: SessionInfo {
                    sessionid: "s1".into(),
                    ..SessionInfo::default()
                },
                ..StartInput::default()
            },
            &mut dm,
        )
        .unwrap();
        let ev = Event::named("go", EventType::External);
        let next = session::step(
            chart,
            start.session.clone(),
            Input::event(ev, Environment::default()),
            &mut dm,
        )
        .unwrap();
        (start, next)
    };
    let (s1, n1) = run(&plain);
    let (s2, n2) = run(&gated);
    assert_eq!(s1, s2);
    assert_eq!(n1, n2);
    assert!(n1.session.is_terminated());
}

// @tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Partially typed chart]] {level: unit}
#[test]
fn partially_typed_chart() {
    let c = chart(
        "event pay: { amount: decimal };",
        vec![],
        vec![
            transition("pay", None, "done", vec![]),
            transition("cancel", None, "done", vec![]),
        ],
    );
    let types = check_chart(&c, &cel()).unwrap();
    let mut pay = Event::named("pay", EventType::External);
    pay.data = obj(&[("amount", s("12.50"))]);
    assert!(types.check_event(&pay).is_ok());
    pay.data = obj(&[("amount", s("lots"))]);
    assert!(types.check_event(&pay).is_err());
    let mut cancel = Event::named("cancel", EventType::External);
    cancel.data = Value::Array(vec![s("anything"), Value::Number(1.0)]);
    assert!(types.check_event(&cancel).is_ok());
    assert_eq!(types.events.payload_of("cancel"), None);
}

// @tg: verifies:: [[openspec:type-system#Dynamic Value Boundary#Assigning dynamic data to typed context]] {level: unit}
#[test]
fn assigning_dynamic_data_to_typed_context() {
    let err = convert(&Type::INT, &s("7")).unwrap_err();
    assert!(err.message.contains("type mismatch"), "{}", err.message);
    assert!(err.message.contains("expected int"), "{}", err.message);
    assert_eq!(
        convert(&Type::INT, &Value::Number(7.0)),
        Ok(Value::Number(7.0))
    );
    // Statically, an untyped context field cannot flow into a typed one without conversion.
    let c = chart(
        "",
        vec![typed_data("n", "int", None), data_item("raw", None)],
        vec![transition(
            "go",
            None,
            "done",
            vec![Exec::new(ExecKind::Assign {
                location: "ctx.n".into(),
                expr: Some("ctx.raw".into()),
                content: None,
            })],
        )],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert!(
        diags[0].message.contains("has type value, expected int"),
        "{}",
        diags[0].message
    );
}

// @tg: verifies:: [[openspec:type-system#Dynamic Value Boundary#Static to dynamic is always allowed]] {level: unit}
#[test]
fn static_to_dynamic_is_always_allowed() {
    let d = Declarations::default();
    for t in [
        Type::INT,
        Type::list(Type::DATETIME),
        Type::record([("a", Type::STRING)]),
        Type::union([("A", None::<Type>)]),
    ] {
        assert!(is_assignable(&d, &t, &Type::Value));
    }
    let v = obj(&[("weird", Value::Undefined)]);
    assert_eq!(convert(&Type::Value, &v), Ok(v.clone()));
    assert_eq!(check_value(&Type::Value, &v), Ok(()));
}

// ============================================================================ events

// @tg: verifies:: [[openspec:type-system#Event Type Declarations#Conflicting declarations for one event]] {level: unit}
#[test]
fn conflicting_declarations_for_one_event() {
    let c = chart(
        "event pay: { amount: decimal }; event pay: { amount: int };",
        vec![],
        vec![transition("pay", None, "done", vec![])],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert_eq!(diags.len(), 1);
    let m = &diags[0].message;
    assert!(m.contains("`pay`"), "{m}");
    assert!(
        m.contains("{ amount: decimal }") && m.contains("{ amount: int }"),
        "{m}"
    );
    // Repeating an identical declaration is not a conflict.
    assert!(decls("event a: int; event a: int;").validate().is_empty());
}

// @tg: verifies:: [[openspec:type-system#Event Type Declarations#Prefix descriptor sees a union]] {level: unit}
#[test]
fn prefix_descriptor_sees_a_union() {
    let d = decls(
        "event order.created: { id: string, total: decimal }; \
         event order.cancelled: { id: string, reason: string };",
    );
    let table = EventTable::new(&d);
    let seen = table.payload_for_descriptor("order").unwrap();
    assert_eq!(
        seen,
        Type::union([
            ("order.created", d.events[0].payload.clone()),
            ("order.cancelled", d.events[1].payload.clone()),
        ])
    );
    assert_eq!(table.payload_for_descriptor("order.*"), Some(seen));
    assert_eq!(
        table.payload_for_descriptor("order.created"),
        d.events[0].payload.clone()
    );
    // A guard reading a variant-specific field without discriminating is rejected.
    let c = chart(
        "event order.created: { id: string, total: decimal }; \
         event order.cancelled: { id: string, reason: string };",
        vec![],
        vec![transition("order", Some("ev.data.total"), "done", vec![])],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert!(
        diags[0].message.contains("discriminate"),
        "{}",
        diags[0].message
    );
    assert!(descriptor_matches("order", "order.created"));
    assert!(!descriptor_matches("order", "orders"));
}

// @tg: verifies:: [[openspec:type-system#Event Type Declarations#Error events are typed]] {level: unit}
#[test]
fn error_events_are_typed() {
    let table = EventTable::new(&Declarations::default());
    let t = table.payload_for_descriptor("error.execution").unwrap();
    assert_eq!(t, error_payload_type());
    assert_eq!(
        t,
        Type::record([("kind", Type::STRING), ("message", Type::STRING)])
    );
    assert_eq!(table.payload_for_descriptor("error"), Some(t.clone()));
    assert_eq!(table.payload_of("error.communication"), Some(Some(t)));
    assert_eq!(
        table.payload_for_descriptor("done.state.s1"),
        Some(Type::Value)
    );
    // A guard on error.execution may read `message` but not an undeclared field.
    let c = chart(
        "event go;",
        vec![],
        vec![
            transition("error.execution", Some("ev.data.message"), "done", vec![]),
            transition("error.execution", Some("ev.data.code"), "done", vec![]),
        ],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert!(diags[0].message.contains("expected bool"));
    assert!(diags[1].message.contains("unknown field `code`"));
}

#[test]
fn raise_of_a_payload_event_and_typed_params_are_checked() {
    let mut s = send("receipt", &[("total", "'abc'")], 30);
    set_param_type(&mut s.extensions, "total", Some(&Type::DECIMAL));
    let c = chart(
        "event pay: { amount: decimal };",
        vec![],
        vec![transition(
            "go",
            None,
            "done",
            vec![
                Exec::new(ExecKind::Raise {
                    event: "pay".into(),
                }),
                s,
            ],
        )],
    );
    let diags = check_chart(&c, &cel()).unwrap_err();
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert!(diags[0].message.contains("<raise> cannot carry one"));
    assert_eq!(diags[1].kind, DiagnosticKind::Param);
}
