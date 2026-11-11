//! The W3C RDFC-1.0 test suite (`tests/data/rdfc`, see `SOURCE.md` there) run against
//! `vertep_rdf::canon`.

use std::collections::BTreeMap;
use std::path::Path;

use vertep_rdf::RdfFormat;
use vertep_rdf::canon::{CanonError, DEFAULT_MAX_RELATED_GROUP, HashAlgorithm, canonicalize_with};
use vertep_rdf::io::read_dataset;

#[derive(Default, Debug)]
struct Tally {
    passed: Vec<String>,
    failed: Vec<String>,
}

#[test]
fn w3c_rdfc10_suite() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/rdfc");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.jsonld")).unwrap())
            .unwrap();
    let mut tally = Tally::default();
    for entry in manifest["entries"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap().to_owned();
        let kind = entry["type"].as_str().unwrap();
        let algorithm = match entry["hashAlgorithm"].as_str() {
            Some("SHA384") => HashAlgorithm::Sha384,
            _ => HashAlgorithm::Sha256,
        };
        let input = std::fs::read(dir.join(entry["action"].as_str().unwrap())).unwrap();
        let dataset = read_dataset(&input, RdfFormat::NQuads)
            .unwrap_or_else(|e| panic!("{id}: input does not parse: {e}"));
        let result = canonicalize_with(&dataset, algorithm, DEFAULT_MAX_RELATED_GROUP);
        let ok = match kind {
            "rdfc:RDFC10EvalTest" => {
                let expected =
                    std::fs::read_to_string(dir.join(entry["result"].as_str().unwrap())).unwrap();
                match &result {
                    Ok(form) => form.nquads == expected,
                    Err(_) => false,
                }
            }
            "rdfc:RDFC10MapTest" => {
                let expected: BTreeMap<String, String> = serde_json::from_str(
                    &std::fs::read_to_string(dir.join(entry["result"].as_str().unwrap())).unwrap(),
                )
                .unwrap();
                match &result {
                    Ok(form) => form.issued == expected,
                    Err(_) => false,
                }
            }
            "rdfc:RDFC10NegativeEvalTest" => {
                matches!(result, Err(CanonError::PoisonedDataset { .. }))
            }
            other => panic!("{id}: unknown test type {other}"),
        };
        if ok {
            tally.passed.push(id);
        } else {
            tally.failed.push(id);
        }
    }
    println!(
        "RDFC-1.0 suite: {} passed, {} failed {:?}",
        tally.passed.len(),
        tally.failed.len(),
        tally.failed
    );
    assert_eq!(tally.passed.len(), 86, "failed: {:?}", tally.failed);
    assert!(tally.failed.is_empty());
}
