//! Unit tests of the macro pipeline, run on `proc_macro2` tokens outside the compiler.

use std::path::{Path, PathBuf};

use quote::quote;
use vertep_core::ir::{ExecKind, StateElement};

use crate::check::{Level, Locator, check};
use crate::expand_statechart;
use crate::include::{expand_in, load, resolve};
use crate::lower::{SpanTable, lower};
use crate::parse::ChartAst;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_dir() -> PathBuf {
    manifest().join("../vertep-dm-rust/tests/charts")
}

// ================================================================== Include Chart Macro

// @tg: verifies:: [[openspec:rust-dsl#Include Chart Macro#File change triggers rebuild]] {level: unit}
#[test]
fn file_change_triggers_rebuild() {
    // The expansion includes the file's bytes, so cargo records it in the crate's dep-info and
    // rebuilds the crate when the file changes.
    let out = expand_in(quote!("order.yaml"), &fixture_dir()).to_string();
    let expected = fixture_dir().join("order.yaml").display().to_string();
    assert!(!out.contains("compile_error"), "{out}");
    assert!(
        out.contains(&format!(
            "const _ : & [u8] = :: core :: include_bytes ! ({expected:?})"
        )),
        "{out}"
    );
}

// @tg: verifies:: [[openspec:rust-dsl#Include Chart Macro#Missing file]] {level: unit}
#[test]
fn missing_file() {
    let out = expand_in(quote!("charts/nope.yaml"), &manifest()).to_string();
    let absolute = manifest().join("charts/nope.yaml");
    assert!(absolute.is_absolute());
    assert!(out.contains("compile_error"), "{out}");
    assert!(out.contains(&absolute.display().to_string()), "{out}");
}

#[test]
fn included_files_must_use_the_rust_datamodel() {
    let dir = std::env::temp_dir().join(format!("vertep-macros-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cel.yaml");
    std::fs::write(&path, "vertep: 1\nname: c\nstates:\n  a: {}\n").unwrap();
    let err = load(&path).unwrap_err();
    assert!(err.contains("`cel` datamodel"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        resolve(Path::new("/x"), "/abs/y.scxml"),
        Path::new("/abs/y.scxml")
    );
}

#[test]
fn file_errors_carry_file_line_and_column() {
    let dir = std::env::temp_dir().join(format!("vertep-macros-err-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("bad.yaml"),
        "vertep: 1\nname: c\ndatamodel: rust\nstates:\n  a:\n    on: { go: nowhere }\n",
    )
    .unwrap();
    let out = expand_in(quote!("bad.yaml"), &dir).to_string();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.contains("compile_error"), "{out}");
    assert!(out.contains("nowhere"), "{out}");
}

// ======================================================================= pipeline units

fn lowered(tokens: proc_macro2::TokenStream) -> (vertep_core::ir::Chart, SpanTable) {
    let ast: ChartAst = syn::parse2(tokens).unwrap();
    let mut table = SpanTable::default();
    let l = lower(&ast, &mut table);
    assert!(l.errors.is_empty());
    (l.chart, table)
}

#[test]
fn lowering_covers_states_transitions_and_actions() {
    let (chart, _) = lowered(quote! {
        name: demo,
        binding: late,
        types { Item = { sku: string, qty: int } }
        context { items: list<Item> = [], note: optional<string> = null, n: int = -3 }
        events { add: Item, "order.done", tick }
        initial a;
        state a {
            entry log "hi", raise tick;
            on add if !full => b internal / store, send tick to "#_parent" after "1s";
            on order.done | error.* => b;
            always if In(b) => b;
            invoke "scxml" src "child.scxml" id "kid" autoforward / store;
            data { local: string = "x" }
        }
        state b { history h deep => b1; state b1; state b2; }
    });
    assert_eq!(chart.datamodel, "rust");
    assert!(chart.binding_explicit);
    assert_eq!(chart.data[0].expr.as_deref(), Some("[]"));
    assert_eq!(chart.data[1].expr.as_deref(), Some("null"));
    assert_eq!(chart.data[2].expr.as_deref(), Some("-3"));
    let a = &chart.states[0];
    assert_eq!(a.data[0].expr.as_deref(), Some("\"x\""));
    assert!(matches!(a.onentry[0][0].kind, ExecKind::Log { .. }));
    assert!(matches!(a.onentry[0][1].kind, ExecKind::Raise { .. }));
    let t = &a.transitions[0];
    assert_eq!(t.cond.as_deref(), Some("!vt:guard/demo/full"));
    assert!(t.kind_explicit);
    assert!(
        matches!(&t.body[1].kind, ExecKind::Send(s) if s.target.as_deref() == Some("#_parent") && s.delay.as_deref() == Some("1s"))
    );
    assert_eq!(a.transitions[1].events, ["order.done", "error.*"]);
    assert_eq!(a.transitions[2].cond.as_deref(), Some("In('b')"));
    assert_eq!(a.invokes[0].id.as_deref(), Some("kid"));
    assert!(a.invokes[0].autoforward);
    let h = &chart.states[1].children[0];
    assert!(matches!(h.element, StateElement::History(_)));
}

#[test]
fn spans_map_back_to_tokens_and_then_to_lines() {
    let (chart, table) = lowered(quote! {
        name: s,
        initial a;
        state a;
    });
    let a = &chart.states[0];
    assert_eq!(table.token(a.span).map(|s| s.start().line), Some(1));
    let real = table.real(a.span);
    assert!(real.is_known());
}

#[test]
fn unreachable_states_are_warnings_by_default_and_errors_on_demand() {
    let tokens = quote! {
        name: orders,
        events { ship }
        initial open;
        state open { on ship => shipped; }
        state shipped;
        state archived;
    };
    let (chart, table) = lowered(tokens.clone());
    let r = check(
        &chart,
        None,
        &Locator::Tokens(&table),
        Level::Warn,
        Level::Warn,
    );
    assert!(r.errors.is_empty());
    assert_eq!(r.warnings.len(), 1);
    assert!(r.warnings[0].1.contains("`archived`"));
    let r = check(
        &chart,
        None,
        &Locator::Tokens(&table),
        Level::Deny,
        Level::Warn,
    );
    assert_eq!(r.errors.len(), 1);
    let r = check(
        &chart,
        None,
        &Locator::Tokens(&table),
        Level::Allow,
        Level::Warn,
    );
    assert!(r.warnings.is_empty() && r.errors.is_empty());
    // The expansion carries the warning as a deprecation note.
    let out = expand_statechart(tokens).to_string();
    assert!(
        out.contains("vertep: state `archived` is unreachable"),
        "{out}"
    );
}

#[test]
fn reachability_follows_parallel_regions_history_and_eventless_transitions() {
    let (chart, table) = lowered(quote! {
        name: r,
        initial p;
        parallel p {
            state left { state l1 { always => l2; } state l2 { on back => h; } }
            state right { history h => r2; state r1; state r2; }
        }
    });
    let r = check(
        &chart,
        None,
        &Locator::Tokens(&table),
        Level::Warn,
        Level::Allow,
    );
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // r1 by default entry of the region, l2 eventlessly, r2 through the history default.
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);

    let (chart, table) = lowered(quote! {
        name: r,
        initial p;
        parallel p {
            state left { state l1; state l2; }
            state right { history h => r2; state r1; state r2; }
        }
    });
    let r = check(
        &chart,
        None,
        &Locator::Tokens(&table),
        Level::Warn,
        Level::Allow,
    );
    let named: Vec<&str> = r.warnings.iter().map(|w| w.1.as_str()).collect();
    assert_eq!(named.len(), 2, "{named:?}");
    assert!(
        named[0].contains("`l2`") && named[1].contains("`r2`"),
        "{named:?}"
    );
}

#[test]
fn type_errors_are_compile_errors() {
    let out = expand_statechart(quote! {
        name: t,
        types { A = { x: Missing } }
        initial a;
        state a;
    })
    .to_string();
    assert!(out.contains("unknown type `Missing`"), "{out}");
    let out = expand_statechart(quote! {
        name: t,
        context { n: int = "text" }
        initial a;
        state a;
    })
    .to_string();
    assert!(out.contains("compile_error"), "{out}");
    assert!(out.contains("expected int"), "{out}");
}

#[test]
fn surface_errors_are_reported() {
    let out = expand_statechart(quote! { initial a; state a; }).to_string();
    assert!(out.contains("a chart needs a name"), "{out}");
    let out = expand_statechart(quote! { name: x, datamodel: ecmascript, state a; }).to_string();
    assert!(out.contains("use the `rust` datamodel"), "{out}");
    let out = expand_statechart(quote! { name: x, state a { on go => b c; } state b; }).to_string();
    assert!(out.contains("malformed transition"), "{out}");
}
