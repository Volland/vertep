//! Robustness properties over arbitrary IR values.

use proptest::test_runner::{Config, TestCaseError, TestRunner};
use vertep_scxml::{AnyDatamodel, DeferResolver, ReadOptions, check_chart_round_trip, read, write};
use vertep_testkit::arb_any_chart;

/// Writing never panics; whatever the writer emits either fails to read cleanly or reads to a
/// chart that is itself a fixpoint of write-then-read.
#[test]
fn any_chart_writes_or_fails_cleanly() {
    let mut runner = TestRunner::new(Config {
        cases: 256,
        ..Config::default()
    });
    let resolver = DeferResolver;
    let opts = ReadOptions::new()
        .with_datamodels(&AnyDatamodel)
        .with_resolver(&resolver)
        .check_references(false);
    runner
        .run(&arb_any_chart(), |chart| {
            if let Ok(text) = write(&chart)
                && let Ok(parsed) = read(&text, &opts)
            {
                check_chart_round_trip(&parsed.chart, &opts)
                    .map_err(|e| TestCaseError::fail(format!("{e}")))?;
            }
            Ok(())
        })
        .expect("robust on arbitrary IR");
}
