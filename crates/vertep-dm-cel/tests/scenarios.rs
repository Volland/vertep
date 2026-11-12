//! One test per scenario of the `cel-datamodel` specification, driving the datamodel through
//! the `Datamodel` trait and, where the scenario is about a running chart, through the
//! vertep-core session.

mod common;

use common::*;
use vertep_core::datamodel::{Datamodel, DatamodelRegistry, SessionInfo, Value};
use vertep_core::ir::{Extension, ExtensionValue, Span, VT_NS};
use vertep_core::null::{create_datamodel, start_with_registry};
use vertep_core::session::{Input, Options, StartInput, step};
use vertep_dm_cel::{
    CEL, CelDatamodel, CelRegistry, CelType, CelValue, Decimal, Limits, LoadOptions, Policy,
    TypeEnv, YAML_DEFAULT_DATAMODEL, check_policy, create, load,
};

fn fresh() -> CelDatamodel {
    let mut dm = CelDatamodel::default();
    dm.init_session(&SessionInfo {
        sessionid: "s1".into(),
        name: Some("chart".into()),
        ioprocessors: Value::Undefined,
    })
    .unwrap();
    dm
}

fn none() -> Active {
    Active::of(&[])
}

fn untyped() -> LoadOptions {
    LoadOptions::default()
}

fn typed(env: TypeEnv) -> LoadOptions {
    LoadOptions {
        env,
        ..LoadOptions::default()
    }
}

fn cel_only() -> LoadOptions {
    LoadOptions {
        policy: Policy::CelOnly,
        ..LoadOptions::default()
    }
}

// ----- CEL Datamodel Selection ------------------------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#CEL Datamodel Selection#SCXML chart declares CEL]] {level: unit}
#[test]
fn scxml_chart_declaring_cel_evaluates_cond_expr_and_location_as_cel() {
    // `1u`, single-quoted strings and `size()` are CEL syntax, not ECMAScript.
    let c = chart(
        vec![data("x", "1u + 2u"), data("y", "0u")],
        vec![
            on(
                atomic("idle"),
                then(
                    when(
                        tr("go", "pass"),
                        "ctx.x == 3u && size([1, 2]) == 2 && 'a' + \"b\" == 'ab'",
                    ),
                    vec![assign("ctx.y", "ctx.x * 2u")],
                ),
            ),
            atomic("pass"),
        ],
    );
    let compiled = vertep_core::compile::CompiledChart::new(c).unwrap();
    assert_eq!(compiled.datamodel(), CEL);
    let input = StartInput {
        info: SessionInfo {
            sessionid: "s1".into(),
            name: None,
            ioprocessors: Value::Undefined,
        },
        env: Default::default(),
        options: Options::default(),
        ..StartInput::default()
    };
    let (out, mut dm) = start_with_registry(&compiled, &CelRegistry, input).unwrap();
    assert_eq!(dm.name(), "cel");
    let out = step(
        &compiled,
        out.session,
        Input::event(event("go", Value::Undefined), Default::default()),
        dm.as_mut(),
    )
    .unwrap();
    assert_eq!(out.trace.configuration, vec!["pass".to_string()]);
    assert_eq!(
        dm.eval_value("type(ctx.y) == uint && ctx.y == 6u", &none())
            .unwrap(),
        Value::Bool(true)
    );
    assert!(create("ecmascript").is_none());
    assert!(CelRegistry.create("null").is_none());
}

// @tg: verifies:: [[openspec:cel-datamodel#CEL Datamodel Selection#YAML default]] {level: unit}
#[test]
fn yaml_charts_default_to_the_cel_datamodel() {
    assert_eq!(YAML_DEFAULT_DATAMODEL, "cel");
    let dm = create_datamodel(&CelRegistry, YAML_DEFAULT_DATAMODEL).expect("registered");
    assert_eq!(dm.name(), "cel");
    // An SCXML chart that names no datamodel keeps the SCXML default, `null`.
    assert_eq!(create_datamodel(&CelRegistry, "").unwrap().name(), "null");
}

// ----- CEL Evaluation Scope ---------------------------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#CEL Evaluation Scope#Guard on event data]] {level: unit}
#[test]
fn guard_reads_event_data_and_context() {
    let mut dm = fresh();
    dm.declare("limit").unwrap();
    dm.assign_value("limit", Value::Number(100.0)).unwrap();
    dm.set_event(&event("pay", obj(&[("amount", Value::Number(150.0))])));
    assert!(dm.eval_cond("ev.data.amount > ctx.limit", &none()).unwrap());
    assert!(
        dm.eval_cond(
            "ev.name == 'pay' && ev.type == 'external' && ev.sendid == null",
            &none()
        )
        .unwrap()
    );
    assert!(
        dm.eval_cond("session.id == 's1' && session.name == 'chart'", &none())
            .unwrap()
    );

    // The same guard over a typed context and payload compares exact decimals.
    let env = TypeEnv::typed()
        .with_context_field("limit", CelType::Decimal)
        .with_event(
            "pay",
            CelType::record(Some("Pay"), [("amount", CelType::Decimal)]),
        );
    let mut dm = CelDatamodel::new(env, Limits::default());
    dm.declare("limit").unwrap();
    dm.assign_value("limit", Value::String("100".into()))
        .unwrap();
    dm.set_event(&event(
        "pay",
        obj(&[("amount", Value::String("150.10".into()))]),
    ));
    assert!(dm.eval_cond("ev.data.amount > ctx.limit", &none()).unwrap());
    assert!(
        dm.eval_cond("ev.data.amount - ctx.limit == decimal('50.1')", &none())
            .unwrap()
    );
}

// @tg: verifies:: [[openspec:did-identity#Authenticated Event Origin#Guard accepts only a specific sender]] {level: unit}
#[test]
fn guard_reads_the_verified_origin() {
    let guard = "ev.originverified && ev.origin == 'did:key:z6MkApprover'";
    let mut dm = fresh();
    let approve = event("approve", Value::Undefined);
    dm.set_event(&approve.clone().with_verified_origin("did:key:z6MkApprover"));
    assert!(dm.eval_cond(guard, &none()).unwrap());
    let mut restored = CelDatamodel::default();
    restored.restore(&dm.snapshot()).unwrap();
    assert!(restored.eval_cond(guard, &none()).unwrap());
    // A claimed but unverified origin, and another verified sender, fail the guard.
    let mut claimed = approve.clone();
    claimed.origin = Some("did:key:z6MkApprover".into());
    dm.set_event(&claimed);
    assert!(!dm.eval_cond(guard, &none()).unwrap());
    assert!(dm.eval_cond("ev.originverified == false", &none()).unwrap());
    dm.set_event(&approve.with_verified_origin("did:key:z6MkOther"));
    assert!(!dm.eval_cond(guard, &none()).unwrap());
}

// @tg: verifies:: [[openspec:cel-datamodel#CEL Evaluation Scope#In predicate]] {level: unit}
#[test]
fn in_predicate_answers_from_the_active_configuration() {
    let mut dm = fresh();
    assert!(
        dm.eval_cond("In(\"payment\")", &Active::of(&["checkout", "payment"]))
            .unwrap()
    );
    assert!(
        !dm.eval_cond("In(\"payment\")", &Active::of(&["checkout", "cart"]))
            .unwrap()
    );
    assert!(
        dm.eval_cond("In('cart') || In('payment')", &Active::of(&["cart"]))
            .unwrap()
    );
}

// ----- No Side Effects --------------------------------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#No Side Effects#No clock function]] {level: unit}
#[test]
fn now_does_not_exist() {
    let c = chart(
        vec![],
        vec![on(
            atomic("a"),
            when(tr("go", "a"), "now() > timestamp('2020-01-01T00:00:00Z')"),
        )],
    );
    let errs = load(&c, &untyped()).unwrap_err();
    let text = errs.to_string();
    assert!(
        text.contains("undeclared reference to function `now`"),
        "{text}"
    );
    assert!(text.contains("transition 1 › cond"), "{text}");
    for effectful in [
        "now()",
        "random()",
        "rand()",
        "uuid()",
        "send('x')",
        "env('HOME')",
    ] {
        assert!(
            fresh().eval_value(effectful, &none()).is_err(),
            "{effectful} must not exist"
        );
    }
}

// @tg: verifies:: [[openspec:cel-datamodel#No Side Effects#Assign is the only mutation]] {level: unit}
#[test]
fn a_macrostep_changes_only_the_assigned_location() {
    let c = chart(
        vec![data("a", "1"), data("b", "2"), data("m", "{'k': [1, 2]}")],
        vec![
            on(
                atomic("s"),
                then(
                    when(tr("go", "t"), "ctx.a > 0 && ctx.m.k.all(x, x > 0)"),
                    vec![
                        log("ctx.a + ctx.b"),
                        if_("ctx.m.k.exists(x, x == 2)", vec![log("'two'")]),
                        assign("ctx.b", "ctx.a + 10"),
                    ],
                ),
            ),
            atomic("t"),
        ],
    );
    let mut run = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    let before = run.dm.context().clone();
    run.send("go", Value::Undefined);
    assert_eq!(run.configuration(), vec!["t".to_string()]);
    let (CelValue::Map(b), CelValue::Map(a)) = (before, run.dm.context().clone()) else {
        panic!("context is a map");
    };
    let changed: Vec<String> = b
        .iter()
        .filter(|(k, v)| !a.get(*k).is_some_and(|w| w.cel_eq(v)))
        .map(|(k, _)| format!("{k:?}"))
        .collect();
    assert_eq!(changed, vec![r#"String("b")"#.to_string()]);
    assert_eq!(a.len(), b.len());
    assert!(run.dm.eval_cond("ctx.b == 11", &none()).unwrap());
}

// ----- Guaranteed Termination -------------------------------------------------------------

fn big_list(n: usize) -> Value {
    Value::Array((0..n).map(|i| Value::Number(i as f64)).collect())
}

// @tg: verifies:: [[openspec:cel-datamodel#Guaranteed Termination#Comprehension over a large list]] {level: unit}
#[test]
fn comprehension_over_a_large_list_stops_at_the_step_limit() {
    let limits = Limits {
        max_steps: 10_000,
        ..Limits::default()
    };
    let mut dm = CelDatamodel::new(TypeEnv::untyped(), limits);
    dm.declare("items").unwrap();
    dm.assign_value("items", big_list(20_000)).unwrap();
    let err = dm
        .eval_value("ctx.items.map(x, x * 2)", &none())
        .unwrap_err();
    assert!(err.message.contains("step limit"), "{}", err.message);
    assert!(err.message.contains("10000"), "{}", err.message);
    // A nested comprehension cannot multiply its way past the limit either.
    let err = dm
        .eval_value(
            "ctx.items.exists(x, ctx.items.exists(y, x + y < 0))",
            &none(),
        )
        .unwrap_err();
    assert!(err.message.contains("step limit"), "{}", err.message);
    // Under the limit it completes.
    assert_eq!(
        dm.eval_value("ctx.items.size()", &none()).unwrap(),
        Value::Number(20_000.0)
    );
    dm.assign_value("items", big_list(1_000)).unwrap();
    assert_eq!(
        dm.eval_value("ctx.items.filter(x, x < 3)", &none())
            .unwrap(),
        big_list(3)
    );

    // In a running chart the failed guard raises error.execution.
    let c = chart(
        vec![data("items", "[]")],
        vec![
            on(
                on(
                    atomic("s"),
                    when(tr("go", "ok"), "ctx.items.map(x, x * 2).size() > 0"),
                ),
                tr("error.execution", "failed"),
            ),
            atomic("ok"),
            atomic("failed"),
        ],
    );
    let opts = LoadOptions {
        limits,
        ..LoadOptions::default()
    };
    let mut run = Run::start(c.clone(), load(&c, &opts).unwrap());
    run.dm.assign_value("items", big_list(20_000)).unwrap();
    run.send("go", Value::Undefined);
    assert_eq!(run.configuration(), vec!["failed".to_string()]);
    assert!(run.last_events().contains(&"error.execution".to_string()));
}

// @tg: verifies:: [[openspec:cel-datamodel#Guaranteed Termination#Limits are deterministic]] {level: unit}
#[test]
fn limits_are_exceeded_or_not_identically_every_time() {
    let limits = Limits {
        max_steps: 10_000,
        ..Limits::default()
    };
    let over = "ctx.items.exists(x, ctx.items.exists(y, x + y < 0))";
    let under = "ctx.items.filter(x, x % 7 == 0).map(x, x * x)";
    let mut results = Vec::new();
    for _ in 0..3 {
        let mut dm = CelDatamodel::new(TypeEnv::untyped(), limits);
        dm.declare("items").unwrap();
        dm.assign_value("items", big_list(1_000)).unwrap();
        let a = dm.eval_value(over, &none()).map_err(|e| e.message);
        let a_steps = dm.last_steps();
        let b = dm.eval_value(under, &none()).map_err(|e| e.message);
        let b_steps = dm.last_steps();
        results.push((a, a_steps, b, b_steps));
    }
    assert!(
        results[0].0.is_err(),
        "the long expression exceeds the limit"
    );
    assert!(results[0].2.is_ok(), "the short expression completes");
    assert!(results.iter().all(|r| *r == results[0]), "{results:?}");
}

// ----- Static Type Checking of CEL --------------------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Non-boolean guard]] {level: unit}
#[test]
fn a_non_boolean_guard_fails_loading() {
    let env = TypeEnv::typed().with_context_field("total", CelType::Decimal);
    let c = chart(
        vec![data("total", "decimal('0')")],
        vec![on(atomic("a"), when(tr("go", "a"), "ctx.total + 1"))],
    );
    let errs = load(&c, &typed(env.clone())).unwrap_err();
    assert_eq!(errs.0.len(), 1, "{errs}");
    assert!(
        errs.0[0].message.contains("a guard must have type bool"),
        "{errs}"
    );
    assert!(errs.0[0].message.contains("decimal"), "{errs}");
    assert_eq!(errs.0[0].at, "state `a` › transition 1 › cond");

    let ok = chart(
        vec![data("total", "decimal('0')")],
        vec![on(atomic("a"), when(tr("go", "a"), "ctx.total + 1 > 100"))],
    );
    assert!(load(&ok, &typed(env)).is_ok());
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Assignment type mismatch]] {level: unit}
#[test]
fn an_assignment_of_the_wrong_type_fails_loading() {
    let env = TypeEnv::typed().with_context_field("count", CelType::Int);
    let c = chart(
        vec![data("count", "0")],
        vec![entry(atomic("a"), vec![assign("ctx.count", "'many'")])],
    );
    let errs = load(&c, &typed(env.clone())).unwrap_err();
    let msg = &errs.0[0].message;
    assert!(
        msg.contains("ctx.count") && msg.contains("int") && msg.contains("string"),
        "{msg}"
    );
    // An unknown field is a load error as well.
    let c = chart(
        vec![data("count", "0")],
        vec![entry(atomic("a"), vec![assign("ctx.missing", "1")])],
    );
    let errs = load(&c, &typed(env)).unwrap_err();
    assert!(errs.to_string().contains("no field `missing`"), "{errs}");
}

// @tg: verifies:: [[openspec:cel-datamodel#Static Type Checking of CEL#Untyped chart checks dynamically]] {level: unit}
#[test]
fn untyped_charts_parse_at_load_and_fail_types_at_runtime() {
    // A syntax error is found when the chart loads.
    let bad = chart(vec![data("x", "1 +")], vec![atomic("a")]);
    let errs = load(&bad, &untyped()).unwrap_err();
    assert!(errs.to_string().contains("syntax error"), "{errs}");

    // A type error is not: `int + string` is dynamic until it runs.
    let c = chart(
        vec![data("x", "1"), data("y", "''")],
        vec![
            on(
                on(
                    atomic("s"),
                    then(tr("go", "t"), vec![assign("ctx.y", "ctx.x + 'a'")]),
                ),
                tr("error.execution", "failed"),
            ),
            atomic("t"),
            on(atomic("failed"), tr("never", "s")),
        ],
    );
    let dm = load(&c, &untyped()).expect("dynamic types load");
    let mut run = Run::start(c, dm);
    run.send("go", Value::Undefined);
    assert!(run.last_events().contains(&"error.execution".to_string()));
    let err = run.dm.eval_value("ctx.x + 'a'", &none()).unwrap_err();
    assert!(
        err.message.contains("no matching overload"),
        "{}",
        err.message
    );
}

// ----- Runtime Errors Raise Error Execution -----------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#Runtime Errors Raise Error Execution#Division by zero in a guard]] {level: unit}
#[test]
fn division_by_zero_in_a_guard_is_false_and_raises_error_execution() {
    let c = chart(
        vec![data("total", "100"), data("count", "0")],
        vec![
            on(
                on(
                    atomic("s"),
                    when(tr("go", "high"), "ctx.total / ctx.count > 10"),
                ),
                tr("error.execution", "failed"),
            ),
            atomic("high"),
            atomic("failed"),
        ],
    );
    let mut run = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    run.send("go", Value::Undefined);
    assert_eq!(run.configuration(), vec!["failed".to_string()]);
    assert_eq!(
        run.last_events(),
        vec!["go".to_string(), "error.execution".to_string()]
    );

    let mut dm = load(&c, &untyped()).unwrap();
    for d in ["total", "count"] {
        dm.declare(d).unwrap();
    }
    dm.assign_value("total", Value::Number(100.0)).unwrap();
    dm.assign_value("count", Value::Number(0.0)).unwrap();
    let err = dm
        .eval_cond("ctx.total / ctx.count > 10", &none())
        .unwrap_err();
    assert!(err.message.contains("division by zero"), "{}", err.message);
    assert!(
        err.message.contains("state `s` › transition 1 › cond"),
        "{}",
        err.message
    );
}

// @tg: verifies:: [[openspec:cel-datamodel#Runtime Errors Raise Error Execution#Failed assignment leaves context unchanged]] {level: unit}
#[test]
fn a_failed_assignment_leaves_the_context_unchanged() {
    let mut dm = fresh();
    dm.declare("x").unwrap();
    dm.declare("m").unwrap();
    dm.assign_expr("ctx.x", "5", &none()).unwrap();
    dm.assign_expr("ctx.m", "{'a': {'b': 1}}", &none()).unwrap();
    let before = dm.snapshot();
    assert!(dm.assign_expr("ctx.x", "1 / 0", &none()).is_err());
    assert!(dm.assign_expr("ctx.x", "{'a': 1}.b", &none()).is_err());
    assert!(dm.assign_expr("ctx.m.zzz.b", "2", &none()).is_err());
    assert!(dm.assign_expr("ctx.nope", "2", &none()).is_err());
    assert_eq!(dm.snapshot(), before);
    assert_eq!(dm.eval_value("ctx.x", &none()).unwrap(), Value::Number(5.0));

    // A typed location refuses a value of the wrong type at runtime too.
    let env = TypeEnv::typed().with_context_field("n", CelType::Int);
    let mut dm = CelDatamodel::new(env, Limits::default());
    dm.declare("n").unwrap();
    dm.assign_expr("ctx.n", "7", &none()).unwrap();
    let err = dm.assign_expr("ctx.n", "dyn('7')", &none()).unwrap_err();
    assert!(
        err.message.contains("expected int, found string"),
        "{}",
        err.message
    );
    assert!(dm.eval_cond("ctx.n == 7", &none()).unwrap());

    // The chart sees error.execution.
    let c = chart(
        vec![data("x", "5")],
        vec![
            on(
                on(
                    atomic("s"),
                    then(tr("go", "s2"), vec![assign("ctx.x", "ctx.x / 0")]),
                ),
                tr("error.execution", "failed"),
            ),
            on(atomic("s2"), tr("error.execution", "failed")),
            atomic("failed"),
        ],
    );
    let mut run = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    run.send("go", Value::Undefined);
    assert_eq!(run.configuration(), vec!["failed".to_string()]);
    assert!(run.dm.eval_cond("ctx.x == 5", &none()).unwrap());
}

// ----- Deterministic Evaluation -----------------------------------------------------------

// The "Same result on two targets" scenario is verified by `tests/determinism.rs`.

// @tg: verifies:: [[openspec:cel-datamodel#Deterministic Evaluation#Map iteration order]] {level: unit}
#[test]
fn map_keys_are_visited_in_key_order() {
    let mut dm = fresh();
    let a = dm
        .eval_value("{'b': 1, 'a': 2, 'c': 3}.map(k, k)", &none())
        .unwrap();
    let b = dm
        .eval_value("{'c': 3, 'b': 1, 'a': 2}.map(k, k)", &none())
        .unwrap();
    let abc = Value::Array(
        vec!["a".into(), "b".into(), "c".into()]
            .into_iter()
            .map(Value::String)
            .collect(),
    );
    assert_eq!(a, abc);
    assert_eq!(b, abc);
    // Mixed key types: bools, then ints, then uints, then strings.
    assert_eq!(
        dm.eval_value(
            "{'x': 0, 2: 0, true: 0, 1u: 0, -1: 0}.map(k, string(k))",
            &none()
        )
        .unwrap(),
        Value::Array(
            ["true", "-1", "2", "1", "x"]
                .iter()
                .map(|s| Value::String(s.to_string()))
                .collect()
        )
    );
    // Objects leave the datamodel with sorted keys, whatever order the data arrived in.
    dm.declare("m").unwrap();
    dm.assign_value(
        "m",
        obj(&[("z", Value::Number(1.0)), ("a", Value::Number(2.0))]),
    )
    .unwrap();
    assert_eq!(
        dm.eval_value("ctx.m", &none()).unwrap(),
        obj(&[("a", Value::Number(2.0)), ("z", Value::Number(1.0))])
    );
    // foreach over a map visits its keys in the same order.
    let items = dm
        .foreach_items("{'q': 1, 'b': 2, 'k': 3}", &none())
        .unwrap();
    assert_eq!(
        items,
        vec![
            Value::String("b".into()),
            Value::String("k".into()),
            Value::String("q".into())
        ]
    );
}

// ----- Safe Datamodel for Generated Charts ------------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#Safe Datamodel for Generated Charts#Restricted chart with a script]] {level: unit}
#[test]
fn cel_only_policy_rejects_a_script() {
    let c = chart(vec![], vec![entry(atomic("a"), vec![script("x = 1")])]);
    let errs = load(&c, &cel_only()).unwrap_err();
    assert_eq!(errs.0.len(), 1, "{errs}");
    assert!(errs.0[0].at.contains("<script>"), "{errs}");
    assert!(errs.0[0].message.contains("<script>"), "{errs}");
    // A chart that declares another datamodel is refused as well.
    let mut ecma = c.clone();
    ecma.datamodel = "ecmascript".into();
    ecma.states = vec![atomic("a")];
    let errs = check_policy(&ecma, Policy::CelOnly).unwrap_err();
    assert!(errs[0].message.contains("ecmascript"), "{errs:?}");
}

// @tg: verifies:: [[openspec:cel-datamodel#Safe Datamodel for Generated Charts#Restricted chart with a Rust function reference]] {level: unit}
#[test]
fn cel_only_policy_rejects_rust_function_iris() {
    let c = chart(
        vec![],
        vec![on(atomic("a"), when(tr("go", "a"), "vt:guard/isPaid"))],
    );
    let errs = load(&c, &cel_only()).unwrap_err();
    assert!(
        errs.0
            .iter()
            .any(|e| e.message.contains("`vt:guard/isPaid`")),
        "{errs}"
    );

    // The same IRI in a vt: extension attribute (how a surface may bind a Rust action).
    let mut s = atomic("a");
    let mut t = tr("go", "a");
    t.extensions.push(Extension {
        ns: VT_NS.into(),
        local: "action".into(),
        value: ExtensionValue::Attribute(format!("{VT_NS}action/charge")),
        position: 0,
        span: Span::NONE,
    });
    s.transitions.push(t);
    let errs = check_policy(&chart(vec![], vec![s]), Policy::CelOnly).unwrap_err();
    assert!(
        errs[0].message.contains(&format!("{VT_NS}action/charge")),
        "{errs:?}"
    );
}

// @tg: verifies:: [[openspec:cel-datamodel#Safe Datamodel for Generated Charts#Restricted chart in pure CEL]] {level: unit}
#[test]
fn cel_only_policy_loads_and_runs_a_pure_cel_chart() {
    let c = chart(
        vec![
            data("total", "decimal('0')"),
            data("limit", "decimal('100')"),
        ],
        vec![
            on(
                atomic("cart"),
                then(
                    when(tr("pay", "payment"), "ev.data.amount > 0"),
                    vec![assign("ctx.total", "decimal(ev.data.amount)")],
                ),
            ),
            on(
                on(
                    atomic("payment"),
                    when(tr("authorized", "review"), "ctx.total > ctx.limit"),
                ),
                tr("authorized", "paid"),
            ),
            atomic("review"),
            final_("paid"),
        ],
    );
    let dm = load(&c, &cel_only()).expect("pure CEL passes the policy");
    let mut run = Run::start(c, dm);
    run.send("pay", obj(&[("amount", Value::Number(150.5))]));
    assert_eq!(run.configuration(), vec!["payment".to_string()]);
    run.send("authorized", Value::Undefined);
    assert_eq!(run.configuration(), vec!["review".to_string()]);
    assert!(
        run.dm
            .eval_cond("ctx.total == decimal('150.5')", &none())
            .unwrap()
    );
}

// ----- SCXML Datamodel Contract -----------------------------------------------------------

// @tg: verifies:: [[openspec:cel-datamodel#SCXML Datamodel Contract#Foreach over a list]] {level: unit}
#[test]
fn foreach_binds_item_and_index_in_order() {
    let c = chart(
        vec![
            data("items", "[10, 20, 30]"),
            data("seen", "[]"),
            data("types", "[]"),
        ],
        vec![entry(
            atomic("a"),
            vec![
                foreach(
                    "ctx.items",
                    "it",
                    Some("i"),
                    vec![assign("ctx.seen", "ctx.seen + [[ctx.i, ctx.it]]")],
                ),
                // Items keep their exact CEL type through the interpreter.
                foreach(
                    "[decimal('1.5'), 2.0, 3u, 4]",
                    "ctx.v",
                    None,
                    vec![assign("ctx.types", "ctx.types + [string(type(ctx.v))]")],
                ),
            ],
        )],
    );
    let run = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    let mut dm = run.dm;
    assert!(
        dm.eval_cond("ctx.seen == [[0, 10], [1, 20], [2, 30]]", &none())
            .unwrap()
    );
    assert!(dm.eval_cond("ctx.it == 30 && ctx.i == 2", &none()).unwrap());
    assert_eq!(
        dm.eval_value("ctx.types", &none()).unwrap(),
        Value::Array(
            ["decimal", "double", "uint", "int"]
                .iter()
                .map(|s| Value::String(s.to_string()))
                .collect()
        )
    );
}

// @tg: verifies:: [[openspec:cel-datamodel#SCXML Datamodel Contract#Donedata]] {level: unit}
#[test]
fn donedata_param_reaches_the_done_state_event() {
    let job = compound(
        "job",
        "work",
        vec![
            on(atomic("work"), tr("finish", "fin")),
            donedata(final_("fin"), &[("result", "ctx.total")]),
        ],
    );
    let job = on(
        job,
        then(
            tr("done.state.job", "done"),
            vec![assign("ctx.got", "ev.data.result")],
        ),
    );
    let c = chart(
        vec![data("total", "40 + 2"), data("got", "null")],
        vec![job, atomic("done")],
    );
    let mut run = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    run.send("finish", Value::Undefined);
    assert_eq!(run.configuration(), vec!["done".to_string()]);
    assert!(run.last_events().contains(&"done.state.job".to_string()));
    assert!(
        run.dm
            .eval_cond("ctx.got == 42 && ctx.got == ctx.total", &none())
            .unwrap()
    );
}

// ----- The general datamodel contract, under CEL ------------------------------------------

// @tg: verifies:: [[openspec:datamodel#Datamodel Snapshot#Snapshot and restore]] {level: unit}
#[test]
fn snapshot_and_restore_continue_identically_with_exact_values() {
    let c = chart(
        vec![
            data("n", "0"),
            data("d", "decimal('0.1')"),
            data("f", "0.5"),
            data("u", "1u"),
            data("t", "timestamp('2026-11-11T10:00:00Z')"),
        ],
        vec![on(
            atomic("s"),
            then(
                tr("tick", "s"),
                vec![
                    assign("ctx.n", "ctx.n + 1"),
                    assign("ctx.d", "ctx.d + decimal('0.1')"),
                    assign("ctx.f", "ctx.f * 2.0"),
                    assign("ctx.u", "ctx.u * 3u"),
                    assign("ctx.t", "ctx.t + duration('1h30m')"),
                ],
            ),
        )],
    );
    let mut a = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    for _ in 0..3 {
        a.send("tick", obj(&[("x", Value::Number(1.0))]));
    }
    let snap = a.dm.snapshot();
    let mut b = Run::start(c.clone(), load(&c, &untyped()).unwrap());
    b.dm.restore(&snap).unwrap();
    b.session = a.session.clone();
    a.send("tick", Value::Undefined);
    b.send("tick", Value::Undefined);
    assert_eq!(a.dm.snapshot(), b.dm.snapshot());
    assert_eq!(a.traces.last(), b.traces.last());
    assert!(b.dm.eval_cond(
        "ctx.n == 4 && ctx.d == decimal('0.5') && type(ctx.f) == double && ctx.f == 8.0 && ctx.u == 81u \
         && ctx.t == timestamp('2026-11-11T16:00:00Z')",
        &none()
    ).unwrap());
    assert!(CelDatamodel::default().restore(&Value::Null).is_err());
}

// @tg: verifies:: [[openspec:datamodel#Guard Purity#Read-only condition allowed]] {level: unit}
#[test]
fn conditions_never_change_the_datamodel() {
    let mut dm = fresh();
    dm.declare("x").unwrap();
    dm.assign_value("x", Value::Number(2.0)).unwrap();
    let before = dm.snapshot();
    assert!(dm.eval_cond("ctx.x > 1", &none()).unwrap());
    // There is no assignment operator: a would-be mutation does not even parse.
    assert!(dm.eval_cond("(ctx.x = 5) > 1", &none()).is_err());
    assert!(
        dm.eval_cond("[1, 2].map(y, y * ctx.x).size() == 2", &none())
            .unwrap()
    );
    assert_eq!(dm.snapshot(), before);
}

// @tg: verifies:: [[openspec:datamodel#System Variables#Assignment to a system variable]] {level: unit}
#[test]
fn system_variables_are_readable_and_read_only() {
    let mut dm = fresh();
    dm.set_event(&event("go", obj(&[("n", Value::Number(1.0))])));
    assert!(
        dm.eval_cond(
            "_event.data.n == 1 && _event.name == 'go' && ev.name == 'go'",
            &none()
        )
        .unwrap()
    );
    assert!(
        dm.eval_cond("_sessionid == 's1' && _name == 'chart'", &none())
            .unwrap()
    );
    let before = dm.snapshot();
    for loc in [
        "_sessionid",
        "_event.name",
        "ev.data.n",
        "session.id",
        "ctx",
        "_name",
    ] {
        assert!(
            dm.assign_expr(loc, "'x'", &none()).is_err(),
            "{loc} must be read-only"
        );
        assert!(!dm.is_location(loc), "{loc}");
    }
    assert!(dm.declare("_sessionid").is_err());
    assert_eq!(dm.snapshot(), before);
}

#[test]
fn content_json_and_locations() {
    let mut dm = fresh();
    let v = dm.eval_content(r#"{"a": [1, 2]}"#).unwrap();
    dm.declare("x").unwrap();
    dm.assign_value("x", v).unwrap();
    assert!(dm.eval_cond("ctx.x.a[1] == 2", &none()).unwrap());
    assert_eq!(
        dm.eval_content("  hello \n  world ").unwrap(),
        Value::String("hello world".into())
    );
    assert!(dm.is_location("ctx.x.a"));
    assert!(dm.is_location("ctx.x.newkey"));
    assert!(!dm.is_location("ctx.y"));
    dm.assign_expr("ctx.x.a[0]", "100", &none()).unwrap();
    dm.assign_expr("ctx['x']['b']", "decimal('2.50')", &none())
        .unwrap();
    assert!(
        dm.eval_cond("ctx.x == {'a': [100, 2], 'b': decimal('2.5')}", &none())
            .unwrap()
    );
    assert!(dm.exec_script("x = 1", &none()).is_err());
    let d = dm.get("x.b").unwrap();
    assert!(matches!(d, CelValue::Decimal(v) if v == Decimal::parse("2.5").unwrap()));
}
