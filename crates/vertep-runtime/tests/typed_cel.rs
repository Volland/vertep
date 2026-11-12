//! CEL charts run typed in the actor runtime: a node whose registry offers the CEL datamodel
//! instantiates a chart that declares types with its projected CEL type environment, refuses
//! one with CEL type errors, does not deliver a payload that fails its declared type, and runs
//! an untyped CEL chart dynamically.
mod common;

use std::sync::Arc;

use common::*;
use vertep_core::datamodel::{Datamodel, StateQuery, Value};
use vertep_core::ir::Chart;
use vertep_core::null::WithNull;
use vertep_runtime::{Address, Node, NodeConfig, RegisterError, Shell, VirtualClock};
use vertep_store::MemoryStore;

const ORDER: &str = include_str!("../../../examples/typed-order/order.yaml");

fn node() -> Node {
    let clock = VirtualClock::new(T0);
    let shell = Shell {
        spawn: Arc::new(vertep_runtime::shell::TokioSpawn),
        clock: Arc::new(clock.clone()),
        sleep: Arc::new(clock),
    };
    Node::builder(Arc::new(MemoryStore::new()), shell)
        .datamodels(Arc::new(WithNull(vertep_dm_cel::CelRegistry)))
        .config(NodeConfig::default())
        .build()
}

fn cel_scxml(initial: &str, body: &str) -> Chart {
    chart(&format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="cel" initial="{initial}">{body}</scxml>"#
    ))
}

fn s(v: &str) -> Value {
    Value::String(v.into())
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

/// Whether the CEL expression `expr` is true over the context of `a`.
fn holds(node: &mut Node, a: &Address, expr: &str) -> bool {
    let snapshot = node.state(a).unwrap().datamodel;
    let mut dm = vertep_dm_cel::CelDatamodel::default();
    dm.restore(&snapshot).unwrap();
    matches!(
        dm.evaluate(expr, &NoStates),
        Ok(vertep_dm_cel::CelValue::Bool(true))
    )
}

fn deliver(node: &mut Node, a: &Address, name: &str, data: Value) -> Vec<String> {
    node.send(a, ev_data(name, data)).unwrap();
    node.run_until_idle();
    node.state(a).unwrap().configuration
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL]] {level: integration}
#[test]
fn typed_order_runs_as_an_actor_with_exact_decimals() {
    let mut node = node();
    let order = vertep_yaml::read_str(ORDER).unwrap();
    node.register_type(&ty("order"), order).unwrap();
    let a = va("order", "o-1");
    assert_eq!(
        deliver(&mut node, &a, "add", item(2.0)),
        ["shopping", "cart"]
    );
    assert!(holds(&mut node, &a, "ctx.total == decimal('25.00')"));
    // guard false: qty must be positive
    deliver(&mut node, &a, "add", item(0.0));
    let at_payment = deliver(&mut node, &a, "checkout", Value::Null);
    assert_eq!(at_payment, ["shopping", "payment"]);
    // 20 is less than the total of 2 x 12.50: stay in payment
    let short = deliver(&mut node, &a, "pay", pay("20", card()));
    assert_eq!(short, ["shopping", "payment"]);
    assert_eq!(
        deliver(&mut node, &a, "pay", pay("25", card())),
        ["fulfilment"]
    );
    assert!(holds(
        &mut node,
        &a,
        "size(ctx.items) == 1 && ctx.limit == decimal('500')"
    ));
}

// @tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Partially typed chart]] {level: integration}
#[test]
fn a_payload_that_fails_its_declared_type_is_not_delivered() {
    let mut node = node();
    node.register_type(&ty("order"), vertep_yaml::read_str(ORDER).unwrap())
        .unwrap();
    let a = va("order", "o-2");
    deliver(&mut node, &a, "add", item(2.0));
    deliver(&mut node, &a, "checkout", Value::Null);
    // `Card` without its `value`: the unguarded fallback transition to `fulfilment` would
    // fire if the event were delivered with its unconverted payload.
    let bad = obj(&[("tag", s("Card"))]);
    assert_eq!(
        deliver(&mut node, &a, "pay", pay("25", bad)),
        ["shopping", "payment"]
    );
    assert_eq!(
        deliver(&mut node, &a, "pay", pay("25", card())),
        ["fulfilment"]
    );
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Assignment type mismatch]] {level: integration}
#[test]
fn a_chart_with_cel_type_errors_is_refused() {
    let mut node = node();
    let bad = cel_scxml(
        "a",
        r#"<datamodel><data id="count" vt:type="int" expr="0"/></datamodel>
           <state id="a"><onentry><assign location="ctx.count" expr="'many'"/></onentry></state>"#,
    );
    match node.register_type(&ty("counter"), bad) {
        Err(RegisterError::Refused {
            datamodel,
            problems,
        }) => {
            assert_eq!(datamodel, "cel");
            let all = problems.join("\n");
            assert!(
                all.contains("ctx.count") && all.contains("int") && all.contains("string"),
                "{all}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(
        node.latest(&ty("counter")).is_none(),
        "nothing is registered"
    );
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Untyped chart checks dynamically]] {level: integration}
#[test]
fn an_untyped_cel_chart_runs_dynamically() {
    let mut node = node();
    let tally = cel_scxml(
        "s",
        r#"<datamodel><data id="n" expr="0"/></datamodel>
           <state id="s">
             <transition event="add"><assign location="n" expr="ctx.n + ev.data.k"/></transition>
             <transition event="error.execution" target="failed"/>
           </state>
           <state id="failed"/>"#,
    );
    node.register_type(&ty("tally"), tally).unwrap();
    let a = va("tally", "t-1");
    let k = |v: Value| obj(&[("k", v)]);
    assert_eq!(deliver(&mut node, &a, "add", k(Value::Number(2.0))), ["s"]);
    assert_eq!(deliver(&mut node, &a, "add", k(Value::Number(3.0))), ["s"]);
    assert!(holds(&mut node, &a, "ctx.n == 5"));
    // `int + string` is not refused when the chart loads: it fails when it runs.
    assert_eq!(deliver(&mut node, &a, "add", k(s("x"))), ["failed"]);
    assert!(holds(&mut node, &a, "ctx.n == 5"));
}
