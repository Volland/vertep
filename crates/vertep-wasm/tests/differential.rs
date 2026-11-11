//! The differential records `vertep-wasm` produces (natively here, in WebAssembly in
//! `bindings/js`) are exactly the ones the Rust-versus-Lean differential test hands the Lean
//! oracle, so WebAssembly traces that equal native ones are checked against Lean too.

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use vertep_conformance::differential::{GenInput, arb_case, chart_config, rust_records};
use vertep_conformance::host::HostRegistry;
use vertep_wasm::trace::{DiffCase, DiffInput, diff_records};

#[test]
fn records_are_the_native_differential_records() {
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::from_seed(RngAlgorithm::ChaCha, &[7u8; 32]),
    );
    let strategy = arb_case(chart_config(), 6);
    let mut compared = 0;
    for _ in 0..30 {
        let case = strategy.new_tree(&mut runner).unwrap().current();
        let ours = diff_records(
            &DiffCase {
                chart: case.chart.clone(),
                inputs: case
                    .inputs
                    .iter()
                    .map(|i| match i {
                        GenInput::Event(n) => DiffInput::Event(n.clone()),
                        GenInput::Cancel => DiffInput::Cancel,
                    })
                    .collect(),
            },
            &HostRegistry,
        );
        match rust_records(&case) {
            Ok(theirs) => {
                assert_eq!(ours.error, None);
                assert_eq!(ours.budget_exceeded, theirs.budget_exceeded);
                assert_eq!(ours.records, theirs.records);
                compared += 1;
            }
            Err(_) => assert!(ours.error.is_some()),
        }
    }
    assert!(compared > 10, "{compared} runs compared");
}
