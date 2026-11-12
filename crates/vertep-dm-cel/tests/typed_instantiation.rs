//! The typed load every host goes through: `CelRegistry::create_for` (through
//! `create_datamodel_for`) instantiates a chart that declares types with its projected CEL type
//! environment, refuses one with CEL type errors, and leaves an untyped chart dynamic; the
//! datamodel's `check_event` refuses a payload that fails its declared type.

use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Event, EventType, SessionInfo, StateQuery, Value, admit,
};
use vertep_core::ir::Chart;
use vertep_core::null::{WithNull, create_datamodel_for};
use vertep_core::session::{Input, StartError, StartInput, start, step};
use vertep_dm_cel::{CelRegistry, CelValue};

const ORDER: &str = include_str!("../../../examples/typed-order/order.yaml");

struct NoStates;

impl StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

fn yaml(text: &str) -> Chart {
    vertep_yaml::read_str(text).unwrap_or_else(|e| panic!("chart reads: {e:?}"))
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn obj(kv: &[(&str, Value)]) -> Value {
    Value::Object(
        kv.iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

fn external(name: &str, data: Value) -> Event {
    Event {
        name: name.into(),
        type_: EventType::External,
        data,
        ..Event::default()
    }
}

fn pay(method: Value) -> Event {
    external("pay", obj(&[("amount", s("25")), ("method", method)]))
}

/// Whether `expr` is true over a datamodel snapshot.
fn holds(dm: &dyn Datamodel, expr: &str) -> bool {
    let mut probe = vertep_dm_cel::CelDatamodel::default();
    probe.restore(&dm.snapshot()).unwrap();
    matches!(probe.evaluate(expr, &NoStates), Ok(CelValue::Bool(true)))
}

const TALLY: &str = r#"
vertep: 1
name: tally
datamodel: cel
types:
  Add: { k: decimal }
events:
  add: Add
context:
  total: { type: decimal, init: "0" }
initial: s
states:
  s:
    on:
      add:
        - actions:
            - assign: { location: ctx.total, expr: "ctx.total + ev.data.k" }
"#;

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL]] {level: unit}
#[test]
fn a_typed_chart_is_instantiated_typed_through_the_registry() {
    let chart = CompiledChart::new(yaml(TALLY)).unwrap();
    let mut dm = create_datamodel_for(&WithNull(CelRegistry), chart.chart()).unwrap();
    let out = start(&chart, StartInput::default(), dm.as_mut()).unwrap();
    // the initial value "0" of a decimal field is a decimal, not an int
    assert!(holds(dm.as_ref(), "ctx.total == decimal('0')"));
    let ev = external("add", obj(&[("k", s("0.10"))]));
    let out = step(
        &chart,
        out.session,
        Input::event(ev, Default::default()),
        dm.as_mut(),
    )
    .unwrap();
    let ev = external("add", obj(&[("k", s("0.20"))]));
    step(
        &chart,
        out.session,
        Input::event(ev, Default::default()),
        dm.as_mut(),
    )
    .unwrap();
    // exact: 0.10 + 0.20 is 0.30, and a decimal payload field arrives as a decimal
    assert!(holds(dm.as_ref(), "ctx.total == decimal('0.30')"));
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Non-boolean guard]] {level: unit}
#[test]
fn a_chart_with_cel_type_errors_is_refused_by_the_registry() {
    // The YAML reader's core checker accepts the non-boolean guard; the CEL checker does not.
    let bad = TALLY.replace(
        "- actions:",
        "- cond: \"ctx.total + 1\"\n          actions:",
    );
    let chart = yaml(&bad);
    let err = CelRegistry.check(&chart).unwrap_err();
    assert!(
        err.iter()
            .any(|e| e.contains("a guard must have type bool")),
        "{err:?}"
    );
    match create_datamodel_for(&WithNull(CelRegistry), &chart) {
        Err(StartError::ChartRefused {
            datamodel,
            problems,
        }) => {
            assert_eq!(datamodel, "cel");
            assert_eq!(problems, err);
        }
        Err(e) => panic!("expected a refusal, got {e:?}"),
        Ok(_) => panic!("expected a refusal, got a datamodel"),
    }
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Untyped chart checks dynamically]] {level: unit}
#[test]
fn an_untyped_chart_gets_an_untyped_datamodel() {
    let untyped = r#"
vertep: 1
name: tally
datamodel: cel
initial: s
states:
  s: {}
"#;
    let chart = yaml(untyped);
    assert!(CelRegistry.check(&chart).is_ok());
    let mut dm = create_datamodel_for(&WithNull(CelRegistry), &chart).unwrap();
    dm.init_session(&SessionInfo::default()).unwrap();
    let err = dm.eval_value("1 + 'a'", &NoStates).unwrap_err();
    assert!(err.message.contains("no matching overload"), "{err}");
}

// @tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Partially typed chart]] {level: unit}
#[test]
fn a_payload_that_fails_its_declared_type_is_not_admitted() {
    let chart = yaml(ORDER);
    let dm = create_datamodel_for(&CelRegistry, &chart).unwrap();
    let card = obj(&[("tag", s("Card")), ("value", obj(&[("last4", s("4242"))]))]);
    let good = pay(card);
    assert!(dm.check_event(&good).is_ok());
    assert_eq!(admit(dm.as_ref(), good.clone()), good);

    let bad = pay(obj(&[("tag", s("Card"))]));
    let err = dm.check_event(&bad).unwrap_err();
    assert!(err.message.contains("`pay`"), "{err}");
    let admitted = admit(dm.as_ref(), bad);
    assert_eq!(admitted.name, "error.execution");
    assert_eq!(admitted.type_, EventType::Platform);

    // an undeclared event carries any payload, and platform events are the platform's
    let undeclared = external("audit", obj(&[("anything", Value::Bool(true))]));
    assert!(dm.check_event(&undeclared).is_ok());
    let platform = Event {
        type_: EventType::Platform,
        ..pay(Value::Null)
    };
    assert!(dm.check_event(&platform).is_ok());
}
