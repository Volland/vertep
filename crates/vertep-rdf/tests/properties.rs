//! Property tests over generated charts: round trip identity and robustness of the projection,
//! the shapes and reconstruction on arbitrary IR values.

mod common;

use proptest::prelude::*;
use vertep_rdf::{
    RdfFormat, ReconstructError, project, read_chart, reconstruct, validate, write_chart,
};
use vertep_testkit::{ChartConfig, arb_any_chart_with, arb_chart_with};

fn small() -> ChartConfig {
    ChartConfig {
        max_depth: 2,
        max_top_states: 3,
        max_children: 3,
        max_transitions: 2,
        max_block: 2,
        exec_depth: 1,
        ..ChartConfig::default()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Round Trip Identity over generated well-formed charts (task 6.7).
    #[test]
    fn reconstruct_after_project_is_identity(c in arb_chart_with(small())) {
        prop_assume!(vertep_core::validate::validate(&c).is_ok());
        let p = project(&c);
        let r = reconstruct(&p.graph);
        prop_assert!(r.is_ok(), "{}", r.unwrap_err());
        prop_assert_eq!(&r.unwrap().chart, &c);
        // And through a concrete syntax.
        let back = read_chart(write_chart(&p, RdfFormat::TriG).as_bytes(), RdfFormat::TriG);
        prop_assert_eq!(back.map(|b| b.chart), Ok(c));
    }

    /// On any IR value: projection is total, the verdict of the shapes decides reconstruction,
    /// and whatever reconstructs is the original chart.
    #[test]
    fn any_chart_reconstructs_iff_it_conforms(c in arb_any_chart_with(small())) {
        let p = project(&c);
        let conforms = validate(&p.graph).conforms();
        match reconstruct(&p.graph) {
            Ok(r) => {
                prop_assert!(conforms);
                prop_assert_eq!(r.chart, c);
            }
            Err(ReconstructError::Invalid(report)) => prop_assert!(!conforms && !report.conforms()),
            Err(e) => prop_assert!(false, "a projection never fails to decode: {}", e),
        }
    }
}
