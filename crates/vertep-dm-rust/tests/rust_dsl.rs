//! One test per rust-dsl scenario that runs: charts written with `statechart!` and
//! `include_chart!`, executed through `vertep_core::session::{start, step}` on the `rust`
//! datamodel. Compile-fail scenarios are in `tests/compile.rs`.

use std::sync::Arc;

use vertep_core::compile::CompiledChart;
use vertep_core::datamodel::{Datamodel, Environment, SessionInfo, Value};
use vertep_core::ir::{ExecKind, HistoryType, StateElement, StateKind};
use vertep_core::session::{self, Effect, Input, Outcome, StartInput};
use vertep_dm_rust::{
    ChartEvent, Context, Decimal, Registry, RegistryError, RustDatamodel, StateEnum, lowering,
};

/// The order chart of the design, in all three surfaces. The functions are the same in each.
macro_rules! order_functions {
    () => {
        pub fn positive(_: &Ctx, ev: &Pay) -> bool {
            ev.amount > Decimal::ZERO
        }
        pub fn over_limit(ctx: &Ctx, _: &()) -> bool {
            ctx.total > ctx.limit
        }
        pub fn record_total(ctx: &mut Ctx, ev: &Pay, _: &mut Emit) {
            ctx.total = ev.amount;
        }
    };
}

mod inline {
    use vertep_dm_rust::{Decimal, statechart};

    statechart! {
        name: order,
        datamodel: rust,
        types {
            Pay = { amount: decimal },
            Approval = { by: iri },
        }
        context { total: decimal = 0, limit: decimal = 100 }
        events { start, pay: Pay, authorized, approve: Approval, cancel }

        initial idle;
        state idle { on start => checkout; }
        state checkout {
            initial cart;
            on cancel => cancelled;
            state cart {
                on pay if positive => payment / record_total;
            }
            state payment {
                on authorized if over_limit => review;
                on authorized => paid;
            }
            state review { on approve => paid; }
        }
        final paid;
        final cancelled;
    }

    order_functions!();
}

mod from_scxml {
    use vertep_dm_rust::{Decimal, include_chart};

    include_chart!("tests/charts/order.scxml");

    order_functions!();
}

mod from_yaml {
    use vertep_dm_rust::{Decimal, include_chart};

    include_chart!("tests/charts/order.yaml");

    order_functions!();
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn compiled(chart: &vertep_core::ir::Chart) -> CompiledChart {
    CompiledChart::new(chart.clone()).unwrap()
}

fn start_input(id: &str) -> StartInput {
    StartInput {
        info: SessionInfo {
            sessionid: id.to_string(),
            name: Some("order".to_string()),
            ..SessionInfo::default()
        },
        ..StartInput::default()
    }
}

fn step(
    chart: &CompiledChart,
    s: vertep_core::session::Session,
    event: vertep_core::datamodel::Event,
    dm: &mut dyn Datamodel,
) -> Outcome {
    session::step(chart, s, Input::event(event, Environment::default()), dm).unwrap()
}

// ===================================================================== Statechart Macro

// @tg: verifies:: [[openspec:rust-dsl#Statechart Macro#Equivalent to SCXML]] {level: integration}
#[test]
fn equivalent_to_scxml() {
    let text = include_str!("charts/order.scxml");
    let opts = vertep_scxml::ReadOptions::default().with_datamodels(&vertep_scxml::AnyDatamodel);
    let read = vertep_scxml::read(text, &opts).unwrap().chart;
    assert_eq!(*inline::CHART, read);
    assert_eq!(
        *inline::VERSION,
        vertep_rdf::chart_version(&read).as_str(),
        "same chart version IRI"
    );
    // Spans of the macro chart point into this file.
    assert!(inline::CHART.span.is_known());
    let cart = &inline::CHART.states[1].children[0];
    assert_eq!(cart.id, "cart");
    assert!(cart.span.line > 0 && cart.span.line < 60, "{:?}", cart.span);
}

mod nested {
    use vertep_dm_rust::statechart;

    statechart! {
        name: player,
        events { pause, resume, next, louder, stop }
        initial on;
        parallel on {
            state media {
                initial track1;
                history resume_point deep => track1;
                state track1 { on next => track2; }
                state track2 { state intro { on next => body; } state body; }
            }
            state volume {
                state quiet { on louder => loud; }
                state loud;
            }
            on pause => paused;
        }
        state paused { on resume => resume_point; on stop => off; }
        final off;
    }
}

// @tg: verifies:: [[openspec:rust-dsl#Statechart Macro#Parallel and history states]] {level: integration}
#[test]
fn parallel_and_history_states() {
    let chart = &*nested::CHART;
    let on = &chart.states[0];
    assert_eq!(on.id, "on");
    assert_eq!(on.kind(), StateKind::Parallel);
    let regions: Vec<&str> = on.children.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(regions, ["media", "volume"]);
    let media = &on.children[0];
    let h = &media.children[0];
    assert_eq!(h.id, "resume_point");
    assert_eq!(h.element, StateElement::History(HistoryType::Deep));
    assert_eq!(h.transitions.len(), 1, "the history's default transition");
    assert_eq!(h.transitions[0].targets, ["track1"]);
    assert!(h.transitions[0].events.is_empty() && h.transitions[0].cond.is_none());

    // The typed API mirrors it, and the deep history restores the nested state.
    use nested::{Event, Machine, State};
    assert_eq!(State::On.children(), &[State::Media, State::Volume]);
    let (mut m, _) = Machine::start("p").unwrap();
    assert!(m.is_in(State::Track1) && m.is_in(State::Quiet));
    m.send(Event::Next).unwrap();
    assert!(m.is_in(State::Intro));
    m.send(Event::Pause).unwrap();
    assert!(m.is_in(State::Paused) && !m.is_in(State::On));
    m.send(Event::Resume).unwrap();
    assert!(m.is_in(State::Intro), "deep history restores track2.intro");
    assert!(m.is_in(State::Quiet), "the other region enters its default");
}

// ================================================================== Include Chart Macro

/// Uses every generated name of a chart module with explicit types; compiling it for a module
/// proves the module has that API.
macro_rules! api_shape {
    ($m:ident) => {{
        use $m::*;
        let _: fn(&Ctx, &Pay) -> bool = $m::positive;
        let _: fn() -> Registry<Ctx> = bind;
        let ctx: Ctx = Ctx {
            total: Decimal::ZERO,
            limit: Decimal::ONE,
        };
        let pay: Pay = Pay { amount: dec("1") };
        let approval = Approval {
            by: vertep_dm_rust::Iri::parse("https://example.org/alice").unwrap(),
        };
        let events: Vec<Event> = vec![
            Event::Start,
            Event::Pay { amount: pay.amount },
            Event::Authorized,
            Event::Approve {
                by: approval.by.clone(),
            },
            Event::Cancel,
        ];
        let states: Vec<&'static str> = State::ALL.iter().map(|s| s.id()).collect();
        let names: Vec<&'static str> = events.iter().map(|e| e.name()).collect();
        let _: &str = NAME;
        let _: &vertep_core::ir::Chart = &CHART;
        let _: &String = &VERSION;
        let _: Option<State> = State::Cart.parent();
        let _ = ctx;
        let _: fn(&str) -> Result<(Machine, Vec<Effect>), vertep_dm_rust::MachineError> =
            Machine::start;
        (states, names, (*CHART).clone(), (*VERSION).clone())
    }};
}

// @tg: verifies:: [[openspec:rust-dsl#Include Chart Macro#Same API from SCXML and from the macro]] {level: integration}
#[test]
fn same_api_from_scxml_and_from_the_macro() {
    let a = api_shape!(inline);
    let b = api_shape!(from_scxml);
    assert_eq!(a.0, b.0, "same states");
    assert_eq!(a.1, b.1, "same events");
    assert_eq!(
        a.0,
        [
            "idle",
            "checkout",
            "cart",
            "payment",
            "review",
            "paid",
            "cancelled"
        ]
    );
}

/// Task 7.8: `statechart!`, `include_chart!("order.scxml")` and `include_chart!("order.yaml")`
/// give the same IR, API and version IRI, and run the same.
#[test]
fn order_chart_is_the_same_from_every_surface() {
    let a = api_shape!(inline);
    let b = api_shape!(from_scxml);
    let c = api_shape!(from_yaml);
    assert_eq!(a.2, b.2);
    assert_eq!(a.2, c.2);
    assert_eq!(a.3, b.3);
    assert_eq!(a.3, c.3);
    assert!(a.3.starts_with("urn:vertep:chart:sha256:"));

    fn run<S: vertep_dm_rust::ChartSpec>(events: Vec<S::Event>) -> Vec<&'static str> {
        let (mut m, _) = vertep_dm_rust::Machine::<S>::start("s").unwrap();
        for e in events {
            m.send(e).unwrap();
        }
        m.active().iter().map(|s| s.id()).collect()
    }
    let big = || dec("250");
    assert_eq!(
        run::<inline::Statechart>(vec![
            inline::Event::Start,
            inline::Event::Pay { amount: big() },
            inline::Event::Authorized
        ]),
        ["checkout", "review"]
    );
    assert_eq!(
        run::<from_yaml::Statechart>(vec![
            from_yaml::Event::Start,
            from_yaml::Event::Pay { amount: big() },
            from_yaml::Event::Authorized
        ]),
        ["checkout", "review"]
    );
}

// ================================================================== Generated Typed API

// @tg: verifies:: [[openspec:rust-dsl#Generated Typed API#State enum reflects hierarchy]] {level: integration}
#[test]
fn state_enum_reflects_hierarchy() {
    use inline::{Event, Machine, State};
    assert_eq!(
        State::Checkout.children(),
        &[State::Cart, State::Payment, State::Review]
    );
    assert_eq!(State::Cart.parent(), Some(State::Checkout));
    assert_eq!(State::Checkout.parent(), None);
    assert!(State::Checkout.contains(State::Payment));
    assert!(!State::Cart.contains(State::Checkout));
    assert_eq!(State::Checkout.kind(), StateKind::Compound);

    let (mut m, _) = Machine::start("s").unwrap();
    m.send(Event::Start).unwrap();
    assert!(m.is_in(State::Checkout));
    assert!(m.is_in(State::Cart));
    assert!(!m.is_in(State::Payment));
    assert_eq!(m.active(), [State::Checkout, State::Cart]);
}

// @tg: verifies:: [[openspec:rust-dsl#Generated Typed API#Chart version constant]] {level: integration}
#[test]
fn chart_version_constant() {
    // `vt hash` is the RDFC-1.0 hash of the chart's projection: vertep_rdf::chart_version.
    let expected = vertep_rdf::chart_version(&inline::CHART);
    assert_eq!(*inline::VERSION, expected.as_str());
    assert_eq!(inline::VERSION.len(), "urn:vertep:chart:sha256:".len() + 64);
    // A different chart has a different version.
    assert_ne!(*inline::VERSION, *nested::VERSION);
}

// ===================================================== Compile-Time Reachability and Coverage

mod eventless {
    use vertep_dm_rust::statechart;

    statechart! {
        #[deny(unreachable_states)]
        name: boot,
        context { ready: bool = true }
        initial starting;
        state starting { always if ready => running; }
        state running;
    }

    pub fn ready(ctx: &Ctx, _: &()) -> bool {
        ctx.ready
    }
}

// @tg: verifies:: [[openspec:rust-dsl#Compile-Time Reachability and Coverage Diagnostics#Eventless transitions count toward reachability]] {level: integration}
#[test]
fn eventless_transitions_count_toward_reachability() {
    // `running` is reachable only through the eventless transition; the chart denies
    // unreachable states and still compiles, and the run reaches it.
    let (m, _) = eventless::Machine::start("b").unwrap();
    assert!(m.is_in(eventless::State::Running));
}

// ======================================================================== Rust Datamodel

mod guarded {
    use std::cell::RefCell;

    use vertep_dm_rust::{Decimal, statechart};

    statechart! {
        name: shop,
        types { Pay = { amount: decimal } }
        context { total: decimal = 10, last_event: string, session: string }
        events { pay: Pay }
        initial cart;
        state cart {
            on pay if covers => paid / remember;
            on pay => rejected;
        }
        state paid;
        state rejected;
    }

    thread_local! {
        pub static CALLS: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    }

    pub fn covers(ctx: &Ctx, ev: &Pay) -> bool {
        CALLS.with(|c| {
            c.borrow_mut()
                .push((ctx.total.to_string(), ev.amount.to_string()))
        });
        ev.amount >= ctx.total
    }

    pub fn remember(ctx: &mut Ctx, ev: &Pay, emit: &mut Emit) {
        ctx.total = ev.amount + Decimal::ONE;
        ctx.last_event = emit.sys().event_name().to_string();
        ctx.session = emit.sys().session_id().to_string();
    }
}

fn run_shop(amount: &str) -> (Vec<String>, guarded::Ctx, Vec<(String, String)>) {
    guarded::CALLS.with(|c| c.borrow_mut().clear());
    let chart = compiled(&guarded::CHART);
    let mut dm =
        RustDatamodel::<guarded::Ctx>::for_chart(chart.chart(), Arc::new(guarded::bind())).unwrap();
    let out = session::start(&chart, start_input("sess-7"), &mut dm).unwrap();
    let out = step(
        &chart,
        out.session,
        guarded::Event::Pay {
            amount: dec(amount),
        }
        .to_event(),
        &mut dm,
    );
    let calls = guarded::CALLS.with(|c| c.borrow().clone());
    (out.session.active(&chart), dm.context().clone(), calls)
}

// @tg: verifies:: [[openspec:rust-dsl#Rust Datamodel#Guard evaluation]] {level: integration}
#[test]
fn guard_evaluation() {
    let (active, _, calls) = run_shop("12");
    assert_eq!(active, ["paid"]);
    // The guard got the current context (total 10) and the typed payload (amount 12).
    assert_eq!(calls, [("10".to_string(), "12".to_string())]);

    let (active, ctx, calls) = run_shop("3");
    assert_eq!(
        active,
        ["rejected"],
        "a false guard selects the next transition"
    );
    assert_eq!(calls, [("10".to_string(), "3".to_string())]);
    assert_eq!(ctx.total, dec("10"));
}

// @tg: verifies:: [[openspec:rust-dsl#Rust Datamodel#System variables]] {level: integration}
#[test]
fn system_variables() {
    let (_, ctx, _) = run_shop("12");
    assert_eq!(ctx.last_event, "pay");
    assert_eq!(ctx.session, "sess-7");
    assert_eq!(ctx.total, dec("13"));
}

// ========================================================== Functions Registered by IRI

// @tg: verifies:: [[openspec:rust-dsl#Functions Registered by IRI#Chart loaded from RDF binds functions]] {level: integration}
#[test]
fn chart_loaded_from_rdf_binds_functions() {
    // Store the compiled chart as RDF (Turtle), then load it back at runtime.
    let projected = vertep_rdf::project(&guarded::CHART);
    let turtle = vertep_rdf::write_chart(&projected, vertep_rdf::RdfFormat::Turtle);
    let loaded = vertep_rdf::read_chart(turtle.as_bytes(), vertep_rdf::RdfFormat::Turtle)
        .unwrap()
        .chart;
    assert_eq!(loaded, *guarded::CHART);
    let refs = vertep_dm_rust::chart_functions(&loaded);
    assert_eq!(refs.guards, ["vt:guard/shop/covers"]);
    assert_eq!(refs.actions, ["vt:action/shop/remember"]);

    // The process registered the same functions: every IRI resolves and the chart runs.
    let chart = compiled(&loaded);
    let mut dm =
        RustDatamodel::<guarded::Ctx>::for_chart(&loaded, Arc::new(guarded::bind())).unwrap();
    let out = session::start(&chart, start_input("rdf"), &mut dm).unwrap();
    let out = step(
        &chart,
        out.session,
        guarded::Event::Pay { amount: dec("20") }.to_event(),
        &mut dm,
    );
    assert_eq!(out.session.active(&chart), ["paid"]);
    assert_eq!(dm.context().total, dec("21"));
}

// @tg: verifies:: [[openspec:rust-dsl#Functions Registered by IRI#Unregistered IRI]] {level: integration}
#[test]
fn unregistered_iri() {
    let mut chart = guarded::CHART.clone();
    chart.states[0].transitions[0].cond = Some("vt:guard/isPaid".to_string());
    let err = RustDatamodel::<guarded::Ctx>::for_chart(&chart, Arc::new(guarded::bind()))
        .err()
        .expect("instantiation fails");
    assert_eq!(
        err,
        [RegistryError::Unregistered {
            iri: "vt:guard/isPaid".to_string(),
            role: "guard"
        }]
    );
    assert!(err[0].to_string().contains("vt:guard/isPaid"));
    // The typed session refuses to start too.
    let e = vertep_dm_rust::Machine::<guarded::Statechart>::start_with(
        Arc::new(Registry::new()),
        vertep_dm_rust::Machine::<guarded::Statechart>::start_input("x"),
    )
    .err()
    .unwrap();
    assert!(e.to_string().contains("vt:guard/shop/covers"), "{e}");
}

// @tg: verifies:: [[openspec:rust-dsl#Functions Registered by IRI#Registry rejects duplicates]] {level: unit}
#[test]
fn registry_rejects_duplicates() {
    let mut r = Registry::<guarded::Ctx>::new();
    r.guard::<guarded::Pay, _>("vt:guard/shop/covers", guarded::covers)
        .unwrap();
    let dup = r
        .guard::<(), _>("vt:guard/shop/covers", |_: &guarded::Ctx, _: &()| true)
        .unwrap_err();
    assert_eq!(
        dup,
        RegistryError::Duplicate {
            iri: "vt:guard/shop/covers".to_string()
        }
    );
    assert!(dup.to_string().contains("vt:guard/shop/covers"));
    // The compact and expanded forms name one IRI.
    let expanded = format!("{}guard/shop/covers", vertep_core::ir::VT_NS);
    assert!(
        r.guard::<(), _>(&expanded, |_: &guarded::Ctx, _: &()| true)
            .is_err()
    );
}

// ============================================================================ Pure Guards

// @tg: verifies:: [[openspec:rust-dsl#Pure Guards#Repeated evaluation]] {level: integration}
#[test]
fn repeated_evaluation() {
    let chart = compiled(&guarded::CHART);
    let mut dm =
        RustDatamodel::<guarded::Ctx>::for_chart(chart.chart(), Arc::new(guarded::bind())).unwrap();
    let out = session::start(&chart, start_input("g"), &mut dm).unwrap();
    dm.set_event(&guarded::Event::Pay { amount: dec("11") }.to_event());
    struct NoStates;
    impl vertep_core::datamodel::StateQuery for NoStates {
        fn in_state(&self, _: &str) -> bool {
            false
        }
    }
    let before = dm.snapshot();
    let a = dm.eval_cond("vt:guard/shop/covers", &NoStates).unwrap();
    let b = dm.eval_cond("vt:guard/shop/covers", &NoStates).unwrap();
    assert!(a && b, "same context and event, same result");
    assert_eq!(
        dm.snapshot(),
        before,
        "the guard did not change the datamodel"
    );
    assert!(!dm.eval_cond("!vt:guard/shop/covers", &NoStates).unwrap());
    let _ = out;
}

// ============================================================ Actions Emit Effects as Events

mod effects {
    use vertep_dm_rust::{SendOptions, statechart};

    statechart! {
        // `report` is only sent to the parent, never handled here.
        #[allow(unhandled_events)]
        name: checker,
        types { Report = { ok: bool } }
        events { check, checked, report: Report }
        initial idle;
        state idle {
            on check => checking / verify;
        }
        state checking {
            on checked => verified / notify;
        }
        state verified;
    }

    pub fn verify(_: &mut Ctx, _: &(), emit: &mut Emit) {
        emit.raise(Event::Checked);
    }

    pub fn notify(_: &mut Ctx, _: &(), emit: &mut Emit) {
        emit.send_to("#_parent", Event::Report { ok: true });
        emit.send_with(
            Event::Report { ok: false },
            SendOptions {
                target: Some("#_parent".to_string()),
                delay_ms: 1500,
                ..SendOptions::default()
            },
        );
    }
}

fn run_checker() -> (Outcome, CompiledChart) {
    let chart = compiled(&effects::CHART);
    let mut dm =
        RustDatamodel::<effects::Ctx>::for_chart(chart.chart(), Arc::new(effects::bind())).unwrap();
    let out = session::start(&chart, start_input("c1"), &mut dm).unwrap();
    let out = step(
        &chart,
        out.session,
        effects::Event::Check.to_event(),
        &mut dm,
    );
    (out, chart)
}

// @tg: verifies:: [[openspec:rust-dsl#Actions Emit Effects as Events#Action sends a message]] {level: integration}
#[test]
fn action_sends_a_message() {
    let (out, _) = run_checker();
    let sends: Vec<_> = out
        .effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(
        sends.len(),
        2,
        "both sends are effects of the step: {:?}",
        out.effects
    );
    assert_eq!(sends[0].target.as_deref(), Some("#_parent"));
    assert_eq!(sends[0].event.name, "report");
    assert_eq!(
        sends[0].event.data,
        Value::Object(vec![("ok".to_string(), Value::Bool(true))])
    );
    assert_eq!(sends[0].delay_ms, 0);
    assert_eq!(sends[1].delay_ms, 1500);
    assert_eq!(sends[0].type_, vertep_core::session::SCXML_EVENT_PROCESSOR);
}

// @tg: verifies:: [[openspec:rust-dsl#Actions Emit Effects as Events#Action raises an internal event]] {level: integration}
#[test]
fn action_raises_an_internal_event() {
    let (out, chart) = run_checker();
    // `checked` was raised by `verify` and processed in the same macrostep.
    assert_eq!(out.session.active(&chart), ["verified"]);
    assert!(out.session.internal_queue.is_empty());
    let events: Vec<Option<String>> = out
        .trace
        .microsteps
        .iter()
        .map(|m| m.event.clone())
        .collect();
    assert_eq!(
        events,
        [Some("check".to_string()), Some("checked".to_string())]
    );
}

// ================================================================= lowering conventions

#[test]
fn macro_lowering_matches_the_runtime_conventions() {
    let cart = &inline::CHART.states[1].children[0];
    let t = &cart.transitions[0];
    assert_eq!(t.cond.as_deref(), Some("vt:guard/order/positive"));
    assert_eq!(
        t.body,
        [lowering::action_exec("vt:action/order/record_total")]
    );
    assert!(matches!(t.body[0].kind, ExecKind::Foreach { .. }));
}

#[test]
fn snapshot_and_restore_round_trip_the_context() {
    let chart = compiled(&guarded::CHART);
    let mut dm =
        RustDatamodel::<guarded::Ctx>::for_chart(chart.chart(), Arc::new(guarded::bind())).unwrap();
    let out = session::start(&chart, start_input("snap"), &mut dm).unwrap();
    let _ = step(
        &chart,
        out.session,
        guarded::Event::Pay { amount: dec("40") }.to_event(),
        &mut dm,
    );
    let snap = dm.snapshot();
    let mut fresh = RustDatamodel::<guarded::Ctx>::new(Arc::new(guarded::bind()));
    fresh.restore(&snap).unwrap();
    assert_eq!(fresh.context(), dm.context());
    assert_eq!(fresh.context().total, dec("41"));
    assert!(guarded::Ctx::restore(&Value::Bool(true)).is_err());
}

#[test]
fn wrong_payload_at_runtime_is_an_execution_error() {
    // An untyped external event with a bad payload makes the guard fail: false plus
    // error.execution, never a panic.
    let chart = compiled(&guarded::CHART);
    let mut dm =
        RustDatamodel::<guarded::Ctx>::for_chart(chart.chart(), Arc::new(guarded::bind())).unwrap();
    let out = session::start(&chart, start_input("bad"), &mut dm).unwrap();
    let mut ev = vertep_core::datamodel::Event::named("pay", Default::default());
    ev.data = Value::Object(vec![(
        "amount".to_string(),
        Value::String("ten".to_string()),
    )]);
    let out = step(&chart, out.session, ev, &mut dm);
    assert_eq!(out.session.active(&chart), ["rejected"]);
}
