//! The `statechart!` and `include_chart!` macros: the Rust DSL of Vertep.
//!
//! Both macros run one pipeline inside the compiler: build the chart IR (from tokens, or from an
//! SCXML or YAML file read at compile time), check it with the same well-formedness validator
//! and type checker the runtime uses, report reachability and event coverage, and generate the
//! typed API (`Ctx`, `State`, `Event`, `CHART`, `VERSION`, `bind()`, `Machine`). Every finding
//! is a compile error or warning at the offending tokens.
//!
//! The generated code refers to `::vertep_dm_rust`, which re-exports both macros; depend on
//! `vertep-dm-rust` to use them. The macros never read a store or go through RDF
//! ([[constraints#Proc macros run at compile time]]).
//!
//! @tg: implements:: [[decisions#IR is the hub]]
//! @tg: implements:: [[decisions#One IR type language]]
//! @tg: implements:: [[decisions#Types ride on vt extension nodes]]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

mod check;
mod codegen;
mod conventions;
mod include;
mod ir_tokens;
mod lower;
mod parse;
mod spans;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};

use crate::check::{Level, Locator, check};
use crate::codegen::{Extra, generate};
use crate::lower::{SpanTable, lower};
use crate::parse::ChartAst;

/// Write a statechart inline in Rust tokens.
///
/// ```text
/// statechart! {
///     #[deny(unhandled_events)]
///     name: order,
///     types { Pay = { amount: decimal }, Approval = { by: iri } }
///     context { total: decimal = 0, limit: decimal = 100 }
///     events { start, pay: Pay, authorized, approve: Approval, cancel }
///     initial idle;
///     state idle { on start => checkout; }
///     state checkout {
///         initial cart;
///         on cancel => cancelled;
///         state cart { on pay if positive => payment / record_total; }
///         state payment { on authorized if over_limit => review; on authorized => paid; }
///         state review { on approve => paid; }
///     }
///     final paid;
///     final cancelled;
/// }
/// ```
///
/// See the crate `vertep-dm-rust` for the grammar of states, transitions and actions, and for
/// what the macro generates.
// @tg: implements:: [[openspec:rust-dsl#Statechart Macro]]
#[proc_macro]
pub fn statechart(input: TokenStream) -> TokenStream {
    expand_statechart(input.into()).into()
}

/// Read an SCXML (`.scxml`, `.xml`) or YAML (`.yaml`, `.yml`) chart on the `rust` datamodel at
/// compile time and generate the same typed API `statechart!` generates. The path is relative to
/// the crate's `CARGO_MANIFEST_DIR`; the crate is rebuilt when the file changes.
// @tg: implements:: [[openspec:rust-dsl#Include Chart Macro]]
#[proc_macro]
pub fn include_chart(input: TokenStream) -> TokenStream {
    include::expand(input.into()).into()
}

fn combine(errors: Vec<syn::Error>) -> TokenStream2 {
    let mut it = errors.into_iter();
    let Some(mut first) = it.next() else {
        return TokenStream2::new();
    };
    for e in it {
        first.combine(e);
    }
    first.to_compile_error()
}

fn level(deny: bool, allow: bool) -> Level {
    if deny {
        Level::Deny
    } else if allow {
        Level::Allow
    } else {
        Level::Warn
    }
}

/// The `statechart!` pipeline on `proc_macro2` tokens.
// @tg: implements:: [[openspec:rust-dsl#Statechart Macro]]
pub(crate) fn expand_statechart(input: TokenStream2) -> TokenStream2 {
    let ast: ChartAst = match syn::parse2(input) {
        Ok(a) => a,
        Err(e) => return e.to_compile_error(),
    };
    if ast.name.is_none() {
        return syn::Error::new(
            ast.span,
            "a chart needs a name (`name: order,`); it names the chart and its function IRIs",
        )
        .to_compile_error();
    }
    let mut table = SpanTable::default();
    let lowered = lower(&ast, &mut table);
    if !lowered.errors.is_empty() {
        return combine(lowered.errors);
    }
    let report = check(
        &lowered.chart,
        Some(&lowered.decls),
        &Locator::Tokens(&table),
        level(ast.deny_unreachable, ast.allow_unreachable),
        level(ast.deny_unhandled, ast.allow_unhandled),
    );
    if !report.errors.is_empty() {
        return combine(report.errors);
    }
    let Some(types) = report.types else {
        return syn::Error::new(Span::call_site(), "the chart could not be type-checked")
            .to_compile_error();
    };
    let mut chart = lowered.chart;
    spans::map(&mut chart, &|s| table.real(s));
    generate(
        &chart,
        &types,
        &Extra {
            functions: &table.functions,
            warnings: &report.warnings,
            tracked: None,
            call: Span::call_site(),
        },
    )
}

#[cfg(test)]
mod tests;
