//! Configuration legality as a property test (verification backlog 2.2): on generated
//! well-formed charts, every configuration reachable by `start` and `step` is legal (W3C SCXML
//! 3.11), the Rust counterpart of the Lean Configuration Well-Formedness Theorem.

use proptest::test_runner::{Config, TestCaseError, TestRunner};
use vertep_conformance::HostRegistry;
use vertep_conformance::differential::{GenInput, arb_case};
use vertep_core::bitset::StateSet;
use vertep_core::compile::{CompiledChart, ROOT};
use vertep_core::datamodel::{Environment, Event, EventType, SessionInfo, Value};
use vertep_core::ir::StateKind;
use vertep_core::null::create_datamodel;
use vertep_core::session::{self, Input, InputKind, Options, StartInput};
use vertep_testkit::ChartConfig;

/// Why `config` is not a legal configuration of `cc`, if it is not.
fn illegal(cc: &CompiledChart, config: &StateSet) -> Option<String> {
    let active = config.to_vec();
    let count = |s: usize| {
        cc.state(s)
            .children
            .iter()
            .filter(|c| config.contains(**c))
            .count()
    };
    for &s in &active {
        let st = cc.state(s);
        if s == ROOT || matches!(st.kind, StateKind::History(_)) {
            return Some(format!("{} is not a proper state", st.id));
        }
        let p = st.parent.unwrap_or(ROOT);
        if p != ROOT && !config.contains(p) {
            return Some(format!(
                "{} is active below inactive {}",
                st.id,
                cc.state(p).id
            ));
        }
        match st.kind {
            StateKind::Compound if count(s) != 1 => {
                return Some(format!("{} has {} active children", st.id, count(s)));
            }
            StateKind::Parallel if count(s) != st.children.len() => {
                return Some(format!("{} has an inactive region", st.id));
            }
            _ => {}
        }
    }
    if count(ROOT) != 1 {
        return Some(format!("the root has {} active children", count(ROOT)));
    }
    None
}

fn check(case: &vertep_conformance::differential::GeneratedCase) -> Result<(), String> {
    let Ok(cc) = CompiledChart::new(case.chart.clone()) else {
        return Ok(());
    };
    let Ok(mut dm) = create_datamodel(&HostRegistry, cc.datamodel()) else {
        return Ok(());
    };
    let start = StartInput {
        info: SessionInfo {
            sessionid: "vt0".to_string(),
            name: None,
            ioprocessors: Value::Undefined,
        },
        env: Environment::default(),
        options: Options {
            microstep_budget: 200,
            ..Options::default()
        },
        data: Vec::new(),
        sessions: Some(vec!["vt0".to_string()]),
    };
    let Ok(o) = session::start(&cc, start, dm.as_mut()) else {
        return Ok(());
    };
    let mut s = o.session;
    let mut seen = vec!["start".to_string()];
    for gi in &case.inputs {
        if s.is_terminated() {
            break;
        }
        if let Some(why) = illegal(&cc, &s.configuration) {
            return Err(format!("after {seen:?}: {why}"));
        }
        let kind = match gi {
            GenInput::Event(n) => InputKind::Event(Event::named(n.clone(), EventType::External)),
            GenInput::Cancel => InputKind::Cancel,
        };
        seen.push(format!("{gi:?}"));
        let input = Input {
            env: Environment::default(),
            kind,
            sessions: Some(vec!["vt0".to_string()]),
        };
        match session::step(&cc, s, input, dm.as_mut()) {
            Ok(o) => s = o.session,
            Err(_) => return Ok(()),
        }
    }
    if !s.is_terminated()
        && let Some(why) = illegal(&cc, &s.configuration)
    {
        return Err(format!("after {seen:?}: {why}"));
    }
    Ok(())
}

// @tg: verifies:: [[openspec:formal-model#Configuration Well-Formedness Theorem]] {level: integration}
#[test]
fn reachable_configurations_are_legal() {
    let cases: u32 = std::env::var("VERTEP_LEGALITY_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000);
    let cfg = ChartConfig {
        max_depth: 4,
        max_top_states: 3,
        max_children: 3,
        max_transitions: 2,
        max_targets: 2,
        max_block: 1,
        exec_depth: 1,
        datamodels: vec!["null".to_owned()],
        extensions: false,
        invokes: false,
    };
    let mut runner = TestRunner::new(Config {
        cases,
        failure_persistence: None,
        max_local_rejects: u32::MAX,
        max_global_rejects: u32::MAX,
        ..Config::default()
    });
    let result = runner.run(&arb_case(cfg, 8), |case| {
        check(&case).map_err(TestCaseError::fail)
    });
    if let Err(e) = result {
        panic!("{e}");
    }
}
