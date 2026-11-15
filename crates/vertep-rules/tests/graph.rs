//! The browser build must never reach the native engine. The wasm toolchain cannot build on
//! every machine, so the guarantee is held on the dependency graph.

use std::process::Command;

fn reaches(package: &str, dependency: &str) -> bool {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    Command::new(cargo)
        .args(["tree", "-p", package, "-i", dependency, "-e", "normal"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree runs")
        .status
        .success()
}

// @tg: verifies:: [[openspec:rule-programs#Native Library Is Optional#Browser build]] {level: integration}
#[test]
fn the_browser_build_never_reaches_the_rules_crate_or_the_engine() {
    assert!(!reaches("vertep-wasm", "vertep-rules"));
    assert!(!reaches("vertep-wasm", "leanelpi"));
}

// @tg: verifies:: [[openspec:rule-programs#Native Library Is Optional#Feature off]] {level: integration}
#[cfg(not(feature = "native"))]
#[test]
fn without_the_feature_the_engine_is_not_linked() {
    assert!(!reaches("vertep-rules", "leanelpi"));
}
