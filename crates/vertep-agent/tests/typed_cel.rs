//! CEL charts run typed in the agent host: its default registry instantiates a chart that
//! declares types with its projected CEL type environment, refuses one with CEL type errors at
//! spawn, does not deliver a payload that fails its declared type, and runs an untyped CEL
//! chart dynamically.
mod common;

use common::*;
use vertep_agent::host::{AgentHost, InstantiateError};
use vertep_core::datamodel::{Datamodel, StateQuery, Value};

const ORDER: &str = include_str!("../../../examples/typed-order/order.yaml");

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

fn item(qty: f64) -> Value {
    obj(&[
        ("sku", s("BOOK-1")),
        ("qty", Value::Number(qty)),
        ("price", s("12.50")),
    ])
}

fn pay(amount: &str, method: Value) -> Value {
    obj(&[("amount", s(amount)), ("method", method)])
}

fn card() -> Value {
    obj(&[("tag", s("Card")), ("value", obj(&[("last4", s("4242"))]))])
}

struct NoStates;

impl StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

/// Whether the CEL expression `expr` is true over the context of [`AGENT`].
fn holds(host: &mut AgentHost, expr: &str) -> bool {
    let snapshot = host.state(AGENT).unwrap().datamodel;
    let mut dm = vertep_dm_cel::CelDatamodel::default();
    dm.restore(&snapshot).unwrap();
    matches!(
        dm.evaluate(expr, &NoStates),
        Ok(vertep_dm_cel::CelValue::Bool(true))
    )
}

fn deliver(host: &mut AgentHost, name: &str, data: Value) -> Vec<String> {
    host.send(AGENT, ev_data(name, data));
    host.run_until_idle();
    config(host)
}

fn host() -> AgentHost {
    builder().0.build()
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL]] {level: integration}
#[test]
fn typed_order_runs_in_the_agent_host_with_exact_decimals() {
    let mut host = host();
    spawn(&mut host, vertep_yaml::read_str(ORDER).unwrap());
    assert!(holds(&mut host, "ctx.total == decimal('0')"));
    assert_eq!(deliver(&mut host, "add", item(2.0)), ["shopping", "cart"]);
    assert!(holds(&mut host, "ctx.total == decimal('25.00')"));
    deliver(&mut host, "add", item(0.0));
    assert_eq!(
        deliver(&mut host, "checkout", Value::Null),
        ["shopping", "payment"]
    );
    assert_eq!(
        deliver(&mut host, "pay", pay("20", card())),
        ["shopping", "payment"]
    );
    assert_eq!(deliver(&mut host, "pay", pay("25", card())), ["fulfilment"]);
}

// @tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Partially typed chart]] {level: integration}
#[test]
fn a_payload_that_fails_its_declared_type_is_not_delivered() {
    let mut host = host();
    spawn(&mut host, vertep_yaml::read_str(ORDER).unwrap());
    deliver(&mut host, "add", item(2.0));
    deliver(&mut host, "checkout", Value::Null);
    let bad = obj(&[("tag", s("Card"))]);
    assert_eq!(
        deliver(&mut host, "pay", pay("25", bad)),
        ["shopping", "payment"]
    );
    assert_eq!(deliver(&mut host, "pay", pay("25", card())), ["fulfilment"]);
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Assignment type mismatch]] {level: integration}
#[test]
fn a_chart_with_cel_type_errors_is_refused_at_spawn() {
    let mut host = host();
    let bad = cel(
        "a",
        r#"<datamodel><data id="count" vt:type="int" expr="0"/></datamodel>"#,
        r#"<state id="a"><onentry><assign location="ctx.count" expr="'many'"/></onentry></state>"#,
    );
    let v = host.register(bad).unwrap();
    match host.spawn(AGENT, &v) {
        Err(InstantiateError::Start(e)) => assert!(
            e.contains("ctx.count") && e.contains("int") && e.contains("string"),
            "{e}"
        ),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(host.journal(AGENT).is_empty(), "nothing is journaled");
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Untyped chart checks dynamically]] {level: integration}
#[test]
fn an_untyped_cel_chart_runs_dynamically() {
    let mut host = host();
    spawn(
        &mut host,
        cel(
            "s",
            r#"<datamodel><data id="n" expr="0"/></datamodel>"#,
            r#"<state id="s">
                 <transition event="add"><assign location="n" expr="ctx.n + ev.data.k"/></transition>
                 <transition event="error.execution" target="failed"/>
               </state>
               <state id="failed"/>"#,
        ),
    );
    let k = |v: Value| obj(&[("k", v)]);
    assert_eq!(deliver(&mut host, "add", k(Value::Number(2.0))), ["s"]);
    assert_eq!(deliver(&mut host, "add", k(Value::Number(3.0))), ["s"]);
    assert!(holds(&mut host, "ctx.n == 5"));
    assert_eq!(deliver(&mut host, "add", k(s("x"))), ["failed"]);
}
