//! Round-trip properties: generated charts, typed charts and the SCXML conformance corpus.

use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use vertep_core::ir::{Chart, ExecKind};
use vertep_core::types::chart::{set_data_type, set_declarations, set_param_type, set_send_type};
use vertep_core::types::{
    BasicTyper, Declarations, EventDecl, Field, Primitive, Type, TypeDecl, Variant, check_chart,
    parse_type,
};
use vertep_testkit::{ChartConfig, arb_any_chart, arb_chart, arb_chart_with};
use vertep_yaml::{ReadOptions, WriteError, read, write};

fn round_trip(chart: &Chart) -> Result<(), TestCaseError> {
    let text = write(chart).map_err(|e| TestCaseError::fail(format!("write: {e}")))?;
    let back = read(&text, &ReadOptions::lower_only())
        .map_err(|e| TestCaseError::fail(format!("read:\n{e}\n---\n{text}")))?;
    prop_assert_eq!(&back, chart, "YAML:\n{}", text);
    let again = write(&back).map_err(|e| TestCaseError::fail(format!("rewrite: {e}")))?;
    prop_assert_eq!(again, text, "output is byte-stable");
    Ok(())
}

// ============================================================================ generated charts

proptest! {
    #![proptest_config(Config { cases: 384, ..Config::default() })]

    // @tg: verifies:: [[openspec:yaml-format#Lossless YAML Round Trip]] {level: unit}
    #[test]
    fn well_formed_charts_round_trip(chart in arb_chart()) {
        round_trip(&chart)?;
    }

    #[test]
    fn multi_target_charts_round_trip(chart in arb_chart_with(ChartConfig {
        max_targets: 3,
        datamodels: vec!["null".into(), "cel".into(), "ecmascript".into()],
        ..ChartConfig::default()
    })) {
        round_trip(&chart)?;
    }

    /// Any IR either round-trips or is refused for duplicate state ids; never a silent loss.
    #[test]
    fn any_ir_round_trips_or_is_refused(chart in arb_any_chart()) {
        match write(&chart) {
            Ok(_) => round_trip(&chart)?,
            Err(WriteError::DuplicateStateId(_)) => {}
            Err(e) => prop_assert!(false, "unexpected {e}"),
        }
    }
}

// ============================================================================ typed charts

fn arb_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{0,4}".prop_filter("not reserved", |s| {
        !vertep_core::types::syntax::is_reserved(s)
    })
}

fn arb_label() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => "[a-z][a-z0-9_]{0,5}",
        1 => "[a-z]{1,3}[-. ][a-z]{1,3}",
        1 => Just("oneOf".to_string()),
        1 => Just("int".to_string()),
    ]
}

fn arb_type(names: Vec<String>) -> BoxedStrategy<Type> {
    let leaf = prop_oneof![
        6 => proptest::sample::select(Primitive::ALL.to_vec()).prop_map(Type::Primitive),
        1 => Just(Type::Value),
        2 => proptest::sample::select(if names.is_empty() { vec!["Missing".to_string()] } else { names })
            .prop_map(Type::Named),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            2 => proptest::collection::vec((arb_label(), inner.clone()), 0..4)
                .prop_map(|fs| Type::Record(fs.into_iter().map(|(n, t)| Field::new(n, t)).collect())),
            2 => proptest::collection::vec((arb_label(), proptest::option::of(inner.clone())), 1..4)
                .prop_map(|vs| Type::Union(vs.into_iter().map(|(n, t)| Variant::new(n, t)).collect())),
            1 => inner.clone().prop_map(Type::list),
            1 => inner.prop_map(Type::optional),
        ]
    })
    .boxed()
}

fn arb_declarations() -> impl Strategy<Value = Declarations> {
    proptest::collection::vec(arb_name(), 0..4)
        .prop_flat_map(|mut names| {
            names.sort();
            names.dedup();
            let types = proptest::collection::vec(arb_type(names.clone()), names.len());
            let events = proptest::collection::vec(
                (
                    "[a-z]{1,4}(\\.[a-z]{1,3})?",
                    proptest::option::of(arb_type(names.clone())),
                ),
                0..4,
            );
            (Just(names), types, events)
        })
        .prop_map(|(names, types, events)| Declarations {
            types: names
                .into_iter()
                .zip(types)
                .map(|(name, ty)| TypeDecl {
                    name,
                    ty,
                    span: Default::default(),
                })
                .collect(),
            events: events
                .into_iter()
                .map(|(name, payload)| EventDecl {
                    name,
                    payload,
                    span: Default::default(),
                })
                .collect(),
        })
}

/// Attach declarations and `vt:type` annotations to a generated chart.
fn typed(mut chart: Chart, decls: Declarations, ty: Type) -> Chart {
    set_declarations(&mut chart, Some(&decls));
    if let Some(d) = chart.data.first_mut() {
        set_data_type(d, Some(&ty));
    }
    fn first_send(states: &mut [vertep_core::ir::State], ty: &Type) -> bool {
        for s in states {
            for b in s.onentry.iter_mut() {
                for e in b.iter_mut() {
                    if let ExecKind::Send(send) = &e.kind {
                        let name = send.params.first().map(|p| p.name.clone());
                        set_send_type(e, Some(ty));
                        if let Some(n) = name {
                            set_param_type(&mut e.extensions, &n, Some(ty));
                        }
                        return true;
                    }
                }
            }
            if first_send(&mut s.children, ty) {
                return true;
            }
        }
        false
    }
    first_send(&mut chart.states, &ty);
    chart
}

proptest! {
    #![proptest_config(Config { cases: 256, ..Config::default() })]

    /// The canonical type syntax reads back to the same type.
    #[test]
    fn type_syntax_round_trips(t in arb_type(vec!["Order".into(), "Node".into()])) {
        prop_assert_eq!(parse_type(&t.to_string()), Ok(t.clone()), "{}", t);
    }

    /// Types survive YAML: declarations, data types, send payload types and param types.
    // @tg: verifies:: [[openspec:yaml-format#YAML Types Block]] {level: unit}
    #[test]
    fn typed_charts_round_trip(
        chart in arb_chart(),
        decls in arb_declarations(),
        ty in arb_type(vec!["Order".into()]),
    ) {
        round_trip(&typed(chart, decls, ty))?;
    }
}

// ============================================================================ SCXML corpus

fn scxml_files(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "scxml") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn corpus() -> Vec<(std::path::PathBuf, Chart)> {
    let resolver = vertep_scxml::DeferResolver;
    let opts = vertep_scxml::ReadOptions::new()
        .with_datamodels(&vertep_scxml::AnyDatamodel)
        .with_resolver(&resolver);
    let mut files = scxml_files(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vertep-conformance/suite"
    ));
    files.extend(scxml_files(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vertep-scxml/tests/corpus"
    )));
    assert!(
        files.len() > 200,
        "the conformance suite is present ({} files)",
        files.len()
    );
    files
        .into_iter()
        .map(|p| {
            let text = std::fs::read_to_string(&p).expect("readable");
            let parsed = vertep_scxml::read(&text, &opts)
                .unwrap_or_else(|e| panic!("{}: {e:?}", p.display()));
            (p, parsed.chart)
        })
        .collect()
}

// @tg: verifies:: [[openspec:yaml-format#Lossless YAML Round Trip#SCXML to YAML to IR]] {level: integration}
#[test]
fn scxml_to_yaml_to_ir() {
    for (path, chart) in corpus() {
        let text = write(&chart).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let back = read(&text, &ReadOptions::lower_only())
            .unwrap_or_else(|e| panic!("{}: {e}\n{text}", path.display()));
        assert_eq!(back, chart, "{}:\n{text}", path.display());
        assert_eq!(write(&back).unwrap(), text, "{}: stable", path.display());
    }
}

// @tg: verifies:: [[openspec:type-system#Opt-In Chart Typing#Untyped SCXML chart runs unchanged]] {level: integration}
#[test]
fn conformance_charts_load_untyped() {
    for (path, chart) in corpus() {
        assert!(!vertep_core::types::is_typed(&chart), "{}", path.display());
        let types = check_chart(&chart, &BasicTyper::for_datamodel(&chart.datamodel))
            .unwrap_or_else(|d| panic!("{}: {d:?}", path.display()));
        assert!(!types.typed);
        // The checker leaves the chart untouched, so it compiles to the same chart as before.
        if let Ok(compiled) = vertep_core::compile::CompiledChart::new(chart.clone()) {
            let (gated, _) =
                vertep_core::types::compile_checked(chart.clone(), &BasicTyper::default())
                    .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
            assert_eq!(gated.chart(), compiled.chart());
        }
    }
}

#[test]
fn generated_charts_are_never_lossy() {
    // A deterministic smoke run of the any-IR generator with a fixed seed, so a writer bug
    // shows up in plain `cargo test` output even without proptest's persistence.
    let mut runner = TestRunner::deterministic();
    for _ in 0..64 {
        let chart = arb_any_chart()
            .new_tree(&mut runner)
            .expect("tree")
            .current();
        assert!(!matches!(write(&chart), Err(WriteError::NotLossless)));
    }
}
