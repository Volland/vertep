//! The CEL type environment projection (`type-system` capability, CEL Type Environment
//! Projection; task 2.4 of `openspec/changes/m1-typed-authoring`).

use vertep_cli::cel::{cel_type, type_env};
use vertep_core::ir::Chart;
use vertep_core::types::{BasicTyper, ChartTypes, Type, check_chart, parse_declarations};
use vertep_dm_cel::{CelType, LoadOptions, load};

fn chart(guard: &str) -> Chart {
    let doc = format!(
        r#"vertep: 1
name: limits
datamodel: cel
context:
  total: {{ type: decimal, init: "0" }}
events: {{ check: null }}
initial: a
states:
  a:
    on:
      check:
        - {{ cond: "{guard}", target: b }}
  b: {{ type: final }}
"#
    );
    vertep_yaml::read(&doc, &vertep_yaml::ReadOptions::lower_only())
        .unwrap_or_else(|e| panic!("{e}"))
}

fn types_of(c: &Chart) -> ChartTypes {
    check_chart(c, &BasicTyper::for_datamodel("cel")).unwrap_or_else(|d| panic!("{d:?}"))
}

fn options(types: &ChartTypes) -> LoadOptions {
    LoadOptions {
        env: type_env(types),
        ..LoadOptions::default()
    }
}

// @tg: verifies:: [[openspec:type-system#CEL Type Environment Projection#Field access is checked]] {level: unit}
#[test]
fn field_access_is_checked() {
    let c = chart("ctx.total > 100");
    let types = types_of(&c);
    let env = type_env(&types);
    assert_eq!(env.context_field("total"), Ok(CelType::Decimal));
    // The guard is accepted where CEL requires `bool`...
    load(&c, &options(&types)).unwrap_or_else(|e| panic!("{e}"));
    // ...and the check is typed: the decimal field alone is not a condition.
    let not_bool = chart("ctx.total");
    let errs = load(&not_bool, &options(&types)).expect_err("a decimal is not a bool");
    assert!(!errs.0.is_empty());
}

// @tg: verifies:: [[openspec:type-system#CEL Type Environment Projection#Unknown field fails checking]] {level: unit}
#[test]
fn unknown_field_fails_checking() {
    let types = types_of(&chart("ctx.total > 100"));
    let bad = chart("ctx.missing > 1");
    let errs = load(&bad, &options(&types)).expect_err("`missing` is not declared");
    assert!(
        errs.0.iter().any(|e| e.message.contains("missing")),
        "{errs}"
    );
}

#[test]
fn types_project_to_documented_cel_types() {
    let d = parse_declarations(
        "type Pay = Card { last4: string } | Cash; \
         type Node = { label: string, children: list<Node> }; \
         type At = { when: datetime, who: iri, n: int, ok: bool, any: value, maybe: optional<int> };",
    )
    .unwrap();
    match cel_type(&d, &Type::named("Pay")) {
        CelType::Union(u) => {
            assert_eq!(u.name.as_deref(), Some("Pay"));
            assert_eq!(u.variants.len(), 2);
            assert!(matches!(&u.variants["Card"], Some(CelType::Record(_))));
            assert_eq!(u.variants["Cash"], None);
        }
        other => panic!("{other:?}"),
    }
    match cel_type(&d, &Type::named("At")) {
        CelType::Record(r) => {
            assert_eq!(r.name.as_deref(), Some("At"));
            assert_eq!(r.fields["when"], CelType::Timestamp);
            assert_eq!(r.fields["who"], CelType::String);
            assert_eq!(r.fields["n"], CelType::Int);
            assert_eq!(r.fields["ok"], CelType::Bool);
            assert_eq!(r.fields["any"], CelType::Dyn);
            assert_eq!(r.fields["maybe"], CelType::optional(CelType::Int));
        }
        other => panic!("{other:?}"),
    }
    // A recursive type is expanded once; its recursive occurrence is `dyn`.
    match cel_type(&d, &Type::named("Node")) {
        CelType::Record(r) => assert_eq!(r.fields["children"], CelType::list(CelType::Dyn)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn untyped_charts_get_the_untyped_environment() {
    let c = vertep_yaml::read(
        "vertep: 1\nname: u\ndatamodel: cel\ninitial: a\nstates:\n  a: {}\n",
        &vertep_yaml::ReadOptions::default(),
    )
    .unwrap();
    let env = type_env(&types_of(&c));
    assert!(!env.is_typed());
    assert_eq!(env.context_field("anything"), Ok(CelType::Dyn));
}
