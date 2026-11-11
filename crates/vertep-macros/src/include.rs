//! `include_chart!`: read an SCXML or YAML chart at compile time and run the macro pipeline.

use std::path::{Path, PathBuf};

use proc_macro2::{Span, TokenStream};
use syn::LitStr;
use vertep_core::ir::Chart;

use crate::check::{Level, Locator, check};
use crate::codegen::{Extra, generate};

/// Resolve `rel` against `manifest_dir`.
pub fn resolve(manifest_dir: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        manifest_dir.join(p)
    }
}

/// Read a chart file, choosing the reader by extension. Errors carry file, line and column.
// @tg: implements:: [[openspec:rust-dsl#Include Chart Macro]]
pub fn load(path: &Path) -> Result<Chart, String> {
    let shown = path.display().to_string();
    if !path.exists() {
        return Err(format!("include_chart!: no chart file at `{shown}`"));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("include_chart!: cannot read `{shown}`: {e}"))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let chart = match ext.as_str() {
        "scxml" | "xml" => {
            let opts = vertep_scxml::ReadOptions::default()
                .with_datamodels(&vertep_scxml::AnyDatamodel)
                .strict(true);
            vertep_scxml::read(&text, &opts)
                .map(|p| p.chart)
                .map_err(|e| format!("include_chart!: `{shown}`: {e}"))?
        }
        "yaml" | "yml" => {
            let opts = vertep_yaml::ReadOptions::lower_only().with_file(&shown);
            vertep_yaml::read(&text, &opts).map_err(|e| format!("include_chart!: {e}"))?
        }
        _ => {
            return Err(format!(
                "include_chart!: `{shown}` is neither SCXML (.scxml, .xml) nor YAML (.yaml, .yml)"
            ));
        }
    };
    if chart.datamodel != "rust" {
        return Err(format!(
            "include_chart!: `{shown}` uses the `{}` datamodel; the generated API binds Rust \
             functions, so the chart must declare datamodel `rust`",
            chart.datamodel
        ));
    }
    if chart.name.is_none() {
        return Err(format!(
            "include_chart!: `{shown}` has no chart name; it names the chart and its function IRIs"
        ));
    }
    Ok(chart)
}

/// Expand `include_chart!("path")` relative to `manifest_dir`.
pub fn expand_in(input: TokenStream, manifest_dir: &Path) -> TokenStream {
    let lit: LitStr = match syn::parse2(input) {
        Ok(l) => l,
        Err(e) => {
            return syn::Error::new(
                e.span(),
                "expected a path string: include_chart!(\"order.scxml\")",
            )
            .to_compile_error();
        }
    };
    let call = lit.span();
    let path = resolve(manifest_dir, &lit.value());
    let chart = match load(&path) {
        Ok(c) => c,
        Err(msg) => return syn::Error::new(call, msg).to_compile_error(),
    };
    let shown = path.display().to_string();
    let loc = Locator::File {
        path: shown.clone(),
        call,
    };
    let report = check(&chart, None, &loc, Level::Warn, Level::Warn);
    if !report.errors.is_empty() {
        let mut it = report.errors.into_iter();
        let mut first = it.next().unwrap_or_else(|| syn::Error::new(call, "error"));
        for e in it {
            first.combine(e);
        }
        return first.to_compile_error();
    }
    let Some(types) = report.types else {
        return syn::Error::new(call, "the chart could not be type-checked").to_compile_error();
    };
    let functions = Default::default();
    generate(
        &chart,
        &types,
        &Extra {
            functions: &functions,
            warnings: &report.warnings,
            tracked: Some(shown),
            call: Span::call_site().located_at(call),
        },
    )
}

/// `include_chart!` relative to the invoking crate's `CARGO_MANIFEST_DIR`.
pub fn expand(input: TokenStream) -> TokenStream {
    let dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_default();
    expand_in(input, &dir)
}
