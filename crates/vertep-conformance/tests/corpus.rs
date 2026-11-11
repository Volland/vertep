//! Round-trip and shape checks over the committed IRP corpus.

use vertep_conformance::corpus::{check_corpus, check_rdf, corpus_files};
use vertep_conformance::manifest::default_suite_dir;
use vertep_scxml::{AnyDatamodel, DeferResolver, ReadOptions, read};

// @tg: verifies:: [[openspec:scxml-conformance#Round Trip Corpus Checks#Corpus round trips]] {level: integration}
#[test]
fn conformance_corpus_round_trips() {
    let report = check_corpus(&default_suite_dir());
    assert!(
        report.checked >= 237,
        "corpus has {} charts",
        report.checked
    );
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
}

// @tg: verifies:: [[openspec:scxml-conformance#Round Trip Corpus Checks#Corpus conforms to shapes]] {level: integration}
#[test]
fn conformance_corpus_conforms_to_shapes() {
    let resolver = DeferResolver;
    let opts = ReadOptions::new()
        .with_datamodels(&AnyDatamodel)
        .with_resolver(&resolver);
    let files = corpus_files(&default_suite_dir());
    assert!(!files.is_empty());
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let chart = read(&text, &opts).expect("reads").chart;
        let graph = vertep_rdf::project(&chart).graph;
        let report = vertep_rdf::validate(&graph);
        assert!(report.conforms(), "{}: {report:?}", path.display());
        check_rdf(&chart).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
}
