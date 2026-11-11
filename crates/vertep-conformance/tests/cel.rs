//! The datamodel-neutral IRP tests under the CEL datamodel (M1 task 6.10).
//!
//! `suite/cel/` holds the `confCel.xsl` variants of every IRP template that needs nothing
//! ECMAScript-specific; `suite/cel/EXCLUDED.tsv` says why each other test has none. Every
//! mandatory CEL variant must pass or be allow-listed (in the `cel` section of
//! `allow-list.toml`); the run writes `target/conformance-cel-report.json`.

use vertep_conformance::allow::default_allow_list_path;
use vertep_conformance::{
    AllowList, Classification, Datamodel, RustCore, Suite, cel_exclusions, report_path, run_suite,
};

// @tg: verifies:: [[openspec:cel-datamodel#SCXML Datamodel Contract#Foreach over a list]] {level: integration}
// @tg: verifies:: [[openspec:cel-datamodel#SCXML Datamodel Contract#Donedata]] {level: integration}
#[test]
fn conformance_cel_suite() {
    let suite = Suite::committed().expect("the committed suite reads");
    let allow =
        AllowList::load(&default_allow_list_path(), &suite).expect("the allow-list is valid");
    let cases: Vec<_> = suite
        .cases()
        .into_iter()
        .filter(|c| c.datamodel == Datamodel::Cel)
        .collect();
    assert!(!cases.is_empty(), "the suite has cel variants");
    let mut report = run_suite(&cases, &allow, &RustCore::default());
    report.excluded = cel_exclusions(&suite).expect("cel/EXCLUDED.tsv reads");
    let path = report_path().with_file_name("conformance-cel-report.json");
    report.write_json(&path).expect("write the report");
    println!("{}", report.summary());
    println!("report: {}", path.display());
    assert!(
        report.gate_passes(),
        "CEL conformance gate failed:\n{}",
        report.summary()
    );
}

/// Every manifest test has a cel variant or a recorded reason for not having one, never both,
/// and only templates (tests leaving the datamodel to `conf:datamodel`) have variants.
#[test]
fn conformance_cel_variants_cover_every_template() {
    let suite = Suite::committed().unwrap();
    let excluded = suite.cel_exclusions().unwrap();
    for t in &suite.tests {
        let has = suite.has_variant(&t.id, Datamodel::Cel);
        let reason = excluded.iter().find(|(id, _)| *id == t.id);
        assert!(
            has != reason.is_some(),
            "{}: a cel variant ({has}) or an exclusion ({reason:?}), exactly one",
            t.id
        );
        if let Some((_, r)) = reason {
            assert!(
                !r.trim().is_empty(),
                "{} is excluded without a reason",
                t.id
            );
        }
        if has {
            let raw = std::fs::read_to_string(suite.root.join("raw").join(&t.starts[0]))
                .expect("the raw template");
            assert!(raw.contains("conf:datamodel"), "{} is not a template", t.id);
            let case = suite.case(t, Datamodel::Cel).unwrap();
            for start in &case.starts {
                let text = without_comments(&std::fs::read_to_string(start).unwrap());
                assert!(text.contains(r#"datamodel="cel""#), "{}", start.display());
                // W3C comments mention `conf:` attributes by name; only markup must be free of them.
                assert!(!text.contains("conf:"), "{}", start.display());
            }
        }
    }
    // The mandatory templates without a cel variant are exactly those needing <script>.
    let mandatory_excluded: Vec<&str> = excluded
        .iter()
        .filter(|(id, r)| {
            suite.test(id).map(|t| t.classification()) == Some(Classification::Mandatory)
                && !r.starts_with("not a template")
        })
        .map(|(id, _)| id.as_str())
        .collect();
    for id in &mandatory_excluded {
        let (_, r) = excluded.iter().find(|(t, _)| t == id).unwrap();
        assert!(r.contains("<script>"), "{id}: {r}");
    }
}

/// The text with every XML comment removed.
fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("-->")
            .map_or("", |end| &rest[start + end + 3..]);
    }
    out.push_str(rest);
    out
}
