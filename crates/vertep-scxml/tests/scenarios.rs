//! One test per scenario of the `scxml-format` capability.

use proptest::test_runner::{Config, TestCaseError, TestRunner};
use quick_xml::Reader;
use quick_xml::events::Event;
use vertep_core::ir::{
    Chart, ExecKind, Extension, ExtensionValue, HistoryType, SCXML_NS, State, StateElement,
    StateKind, Transition, VT_NS,
};
use vertep_scxml::{
    AnyDatamodel, DeferResolver, ErrorKind, LossKind, MapResolver, ReadError, ReadOptions,
    check_chart_round_trip, check_round_trip, read, read_str, write,
};
use vertep_testkit::{ChartConfig, arb_chart_with};

fn ecma() -> ReadOptions<'static> {
    static DMS: [&str; 2] = ["null", "ecmascript"];
    ReadOptions::new().with_datamodels(&DMS)
}

fn errors_of(r: Result<vertep_scxml::Parsed, ReadError>) -> Vec<vertep_scxml::ParseError> {
    match r {
        Ok(p) => panic!("expected errors, got a chart: {:#?}", p.chart),
        Err(e) => e.errors().to_vec(),
    }
}

fn find<'c>(states: &'c [State], id: &str) -> &'c State {
    fn go<'c>(states: &'c [State], id: &str) -> Option<&'c State> {
        for s in states {
            if s.id == id {
                return Some(s);
            }
            if let Some(f) = go(&s.children, id) {
                return Some(f);
            }
        }
        None
    }
    go(states, id).unwrap_or_else(|| panic!("no state `{id}`"))
}

/// Every `.scxml` file below `dir` (none when it does not exist).
fn scxml_files(dir: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "scxml") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Remove every extension node from a chart (what a processor ignoring `vt:` would see).
fn strip_extensions(chart: &mut Chart) {
    fn block(b: &mut Vec<vertep_core::ir::Exec>) {
        b.retain(|x| !matches!(x.kind, ExecKind::Extension(_)));
        for x in b {
            x.extensions.clear();
            match &mut x.kind {
                ExecKind::If { branches } => branches.iter_mut().for_each(|br| block(&mut br.body)),
                ExecKind::Foreach { body, .. } => block(body),
                _ => {}
            }
        }
    }
    fn state(s: &mut State) {
        s.extensions.clear();
        for t in &mut s.transitions {
            t.extensions.clear();
            block(&mut t.body);
        }
        s.onentry.iter_mut().for_each(block);
        s.onexit.iter_mut().for_each(block);
        s.children.iter_mut().for_each(state);
    }
    chart.extensions.clear();
    chart.states.iter_mut().for_each(state);
}

// ------------------------------------------------------------------ SCXML Document Parsing

const VALID: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"
       datamodel="ecmascript" initial="main">
  <datamodel>
    <data id="x" expr="1"/>
    <data id="y">{"k": 2}</data>
  </datamodel>
  <state id="main">
    <transition event="go" target="p"/>
    <transition event="stop" target="done"/>
  </state>
  <parallel id="p">
    <state id="r1">
      <state id="r1a"/>
      <history id="h" type="deep">
        <transition target="r1a"/>
      </history>
    </state>
    <state id="r2">
      <invoke type="scxml" id="child" autoforward="true">
        <param name="a" expr="x"/>
      </invoke>
    </state>
  </parallel>
  <final id="done"/>
</scxml>"#;

// @tg: verifies:: [[openspec:scxml-format#SCXML Document Parsing#Valid document parsed]] {level: unit}
#[test]
fn valid_document_parsed() {
    let chart = read(VALID, &ecma()).expect("valid document").chart;
    assert_eq!(chart.datamodel, "ecmascript");
    assert_eq!(chart.initial, ["main"]);
    let ids: Vec<&str> = chart.data.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, ["x", "y"]);
    assert_eq!(chart.data[0].expr.as_deref(), Some("1"));
    assert_eq!(chart.data[1].content.as_deref(), Some("{\"k\": 2}"));

    let top: Vec<&str> = chart.states.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        top,
        ["main", "p", "done"],
        "document order of top-level states"
    );
    let main = &chart.states[0];
    assert_eq!(main.kind(), StateKind::Atomic);
    let events: Vec<&[String]> = main
        .transitions
        .iter()
        .map(|t| t.events.as_slice())
        .collect();
    assert_eq!(events, [["go".to_owned()], ["stop".to_owned()]]);
    assert_eq!(main.transitions[0].targets, ["p"]);

    let p = &chart.states[1];
    assert_eq!(p.kind(), StateKind::Parallel);
    assert_eq!(p.children.len(), 2);
    let r1 = find(&chart.states, "r1");
    assert_eq!(r1.kind(), StateKind::Compound);
    let h = find(&chart.states, "h");
    assert_eq!(h.kind(), StateKind::History(HistoryType::Deep));
    assert_eq!(h.transitions[0].targets, ["r1a"]);
    let r2 = find(&chart.states, "r2");
    let inv = &r2.invokes[0];
    assert_eq!(inv.type_.as_deref(), Some("scxml"));
    assert_eq!(inv.id.as_deref(), Some("child"));
    assert!(inv.autoforward);
    assert_eq!(inv.params[0].name, "a");
    assert_eq!(inv.params[0].expr.as_deref(), Some("x"));
    assert_eq!(chart.states[2].kind(), StateKind::Final);
}

// @tg: verifies:: [[openspec:scxml-format#SCXML Document Parsing#Wrong namespace rejected]] {level: unit}
#[test]
fn wrong_namespace_rejected() {
    let doc =
        r#"<scxml xmlns="http://example.org/not-scxml" version="1.0"><state id="a"/></scxml>"#;
    let errors = errors_of(read_str(doc));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ErrorKind::WrongNamespace);
    assert!(errors[0].message.contains("http://example.org/not-scxml"));
    assert_eq!((errors[0].span.line, errors[0].span.column), (1, 1));

    let errors = errors_of(read_str(r#"<scxml version="1.0"/>"#));
    assert_eq!(errors[0].kind, ErrorKind::WrongNamespace);
    assert!(errors[0].message.contains("no namespace"));
}

// ------------------------------------------------------------------ SCXML Serialization

// @tg: verifies:: [[openspec:scxml-format#SCXML Serialization#Document written]] {level: unit}
#[test]
fn document_written() {
    let chart = read(VALID, &ecma()).expect("valid").chart;
    let text = write(&chart).expect("writable");
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    let mut reader = Reader::from_str(&text);
    let mut root = None;
    let mut depth = 0i32;
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) | Event::Empty(e) if root.is_none() => {
                let attrs: Vec<(String, String)> = e
                    .attributes()
                    .map(|a| {
                        let a = a.expect("attribute");
                        (
                            a.key.as_ref().to_owned(),
                            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                                .expect("value")
                                .into_owned(),
                        )
                    })
                    .collect();
                root = Some((e.name().as_ref().to_owned(), attrs));
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            Event::Eof => break,
            _ => {}
        }
    }
    assert_eq!(depth, 0, "balanced");
    let (name, attrs) = root.expect("root element");
    assert_eq!(name, "scxml");
    assert!(attrs.contains(&("xmlns".to_owned(), SCXML_NS.to_owned())));
    assert!(attrs.contains(&("version".to_owned(), "1.0".to_owned())));
}

// @tg: verifies:: [[openspec:scxml-format#SCXML Serialization#Generated identifiers not leaked]] {level: unit}
#[test]
fn generated_identifiers_not_leaked() {
    let mut chart = Chart::default();
    let mut a = State::new("a", StateElement::State);
    a.transitions.push(Transition {
        events: vec!["go".to_owned()],
        targets: vec!["_vt.s.2".to_owned()],
        ..Transition::default()
    });
    chart.states.push(a);
    chart.states.push(State::new("", StateElement::State));
    chart.states.push(State::new("", StateElement::Final));
    vertep_core::identity::assign_generated_ids(&mut chart);
    assert_eq!(chart.states[1].id, "_vt.s.1");
    assert_eq!(chart.states[2].id, "_vt.s.2");

    let text = write(&chart).expect("writable");
    assert!(
        !text.contains("_vt.s.1"),
        "untargeted generated id leaked:\n{text}"
    );
    assert!(
        text.contains("<final id=\"_vt.s.2\"/>"),
        "targeted id missing:\n{text}"
    );
    assert!(text.contains("<state/>"));

    let back = read_str(&text).expect("readable").chart;
    assert_eq!(back, chart);
    assert!(
        back.states[2].id_generated,
        "written generated id reads back as generated"
    );
}

// ------------------------------------------------------------------ SCXML Round Trip

// @tg: verifies:: [[openspec:scxml-format#SCXML Round Trip#IR to SCXML to IR]] {level: unit}
#[test]
fn ir_to_scxml_to_ir() {
    let resolver = DeferResolver;
    let opts = ReadOptions::new()
        .with_datamodels(&AnyDatamodel)
        .with_resolver(&resolver);

    // The corpora: this crate's hand-written documents, and the W3C IRP suite imported by
    // vertep-conformance when it is present. Every document reads, and its chart survives
    // write and read.
    let own = scxml_files(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus"));
    assert!(own.len() >= 3, "corpus has documents");
    let irp = scxml_files(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vertep-conformance/suite"
    ));
    for path in own.iter().chain(irp.iter()) {
        let text = std::fs::read_to_string(path).expect("readable");
        if let Err(e) = check_round_trip(&text, &opts) {
            panic!("{}: {e}", path.display());
        }
    }

    // Generated well-formed charts, including multi-target transitions.
    let cfg = ChartConfig {
        max_targets: 3,
        ..ChartConfig::default()
    };
    let mut runner = TestRunner::new(Config {
        cases: 256,
        ..Config::default()
    });
    runner
        .run(&arb_chart_with(cfg), |chart| {
            check_chart_round_trip(&chart, &opts)
                .map(|_| ())
                .map_err(|e| TestCaseError::fail(format!("{e}")))
        })
        .expect("every generated chart round-trips");
}

// @tg: verifies:: [[openspec:scxml-format#SCXML Round Trip#Comments and whitespace do not matter]] {level: unit}
#[test]
fn comments_and_whitespace_do_not_matter() {
    let a = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" name="n" initial="s">
  <state id="s" vt:a="1" vt:b="2">
    <transition event="e" target="t" cond="x"/>
  </state>
  <final id="t"/>
</scxml>"#;
    let b = r#"<?xml version="1.0"?>
<!-- a comment before the root -->
<scxml initial="s"    name="n" version="1.0"
       xmlns:v="https://w3id.org/vertep/vt#" xmlns="http://www.w3.org/2005/07/scxml"><!-- inside -->
<state v:b="2" id="s" v:a="1"><transition cond="x"
   target="t"   event="e"/><!-- between --></state><final id="t"/>
</scxml>
<!-- trailing -->"#;
    let ca = read_str(a).expect("a").chart;
    let pb = read_str(b).expect("b");
    assert_eq!(ca, pb.chart);
    assert!(pb.losses.is_empty(), "comments are formatting, not content");
    assert_eq!(pb.losses.of_kind(LossKind::Comment).count(), 4);
}

// ------------------------------------------------------------------ Parse Error Reporting

// @tg: verifies:: [[openspec:scxml-format#Parse Error Reporting#Mutually exclusive attributes]] {level: unit}
#[test]
fn mutually_exclusive_attributes() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
  <state id="a">
    <onentry>
      <send event="e" eventexpr="'e'"/>
      <raise/>
    </onentry>
  </state>
</scxml>"#;
    let errors = errors_of(read_str(doc));
    let send = errors
        .iter()
        .find(|e| e.kind == ErrorKind::ExclusiveAttributes)
        .expect("exclusive-attribute error");
    assert_eq!((send.span.line, send.span.column), (4, 7));
    assert!(send.message.contains("event") && send.message.contains("eventexpr"));
    // Reading went on after the error and found the next one too.
    let raise = errors
        .iter()
        .find(|e| e.kind == ErrorKind::MissingAttribute)
        .expect("second error reported in the same result");
    assert_eq!((raise.span.line, raise.span.column), (5, 7));
}

// @tg: verifies:: [[openspec:scxml-format#Parse Error Reporting#Missing required attribute]] {level: unit}
#[test]
fn missing_required_attribute() {
    let doc = "<scxml xmlns=\"http://www.w3.org/2005/07/scxml\" version=\"1.0\">\n<final id=\"f\"><donedata>\n  <param expr=\"1\"/>\n</donedata></final></scxml>";
    let errors = errors_of(read_str(doc));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind, ErrorKind::MissingAttribute);
    assert!(errors[0].message.contains("`name`"));
    assert_eq!((errors[0].span.line, errors[0].span.column), (3, 3));
}

// ------------------------------------------------------------------ Source Locations

// @tg: verifies:: [[openspec:scxml-format#Source Locations#Validation error points to the source]] {level: unit}
#[test]
fn validation_error_points_to_the_source() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
  <state id="a">
    <transition event="ok" target="a"/>
        <transition event="go" target="nowhere"/>
  </state>
</scxml>"#;
    let errors = errors_of(read_str(doc));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind, ErrorKind::UnknownTarget);
    assert!(errors[0].message.contains("nowhere"));
    assert_eq!((errors[0].span.line, errors[0].span.column), (4, 9));

    // The span is on the IR node too, for the core validator and runtime diagnostics.
    let chart = read(doc, &ReadOptions::new().check_references(false))
        .expect("readable without reference checks")
        .chart;
    let t = &chart.states[0].transitions[1];
    assert_eq!((t.span.line, t.span.column), (4, 9));
}

// @tg: verifies:: [[openspec:scxml-format#Source Locations#Locations ignored by equality]] {level: unit}
#[test]
fn locations_ignored_by_equality() {
    let a = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="s"><transition event="e" target="s"/></state></scxml>"#;
    let b = "<scxml xmlns=\"http://www.w3.org/2005/07/scxml\"\n       version=\"1.0\">\n\n  <state id=\"s\">\n      <transition event=\"e\" target=\"s\"/>\n  </state>\n</scxml>\n";
    let ca = read_str(a).expect("a").chart;
    let cb = read_str(b).expect("b").chart;
    let (sa, sb) = (
        ca.states[0].transitions[0].span,
        cb.states[0].transitions[0].span,
    );
    assert_ne!((sa.line, sa.column), (sb.line, sb.column));
    assert_eq!((sb.line, sb.column), (5, 7));
    assert_eq!(ca, cb);
}

// ------------------------------------------------------------------ External Content Resolution

// @tg: verifies:: [[openspec:scxml-format#External Content Resolution#No resolver, no access]] {level: unit}
#[test]
fn no_resolver_no_access() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript">
  <script src="lib.js"/>
  <state id="a"/>
</scxml>"#;
    let errors = errors_of(read(doc, &ecma()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ErrorKind::UnresolvedSource);
    assert!(errors[0].message.contains("lib.js"));
    assert_eq!((errors[0].span.line, errors[0].span.column), (2, 3));

    // A `src` naming a file that exists is not read either: the reader has no file access.
    let real = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let doc = format!(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript"><script src="{real}"/><state id="a"/></scxml>"#
    );
    let errors = errors_of(read(&doc, &ecma()));
    assert_eq!(errors[0].kind, ErrorKind::UnresolvedSource);
}

// @tg: verifies:: [[openspec:scxml-format#External Content Resolution#Resolver supplies content]] {level: unit}
#[test]
fn resolver_supplies_content() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="ecmascript">
  <script src="lib.js"/>
  <state id="a"><onentry><script src="lib.js"/></onentry></state>
</scxml>"#;
    let resolver = MapResolver::new().with("lib.js", "function f() { return 1; }");
    let chart = read(doc, &ecma().with_resolver(&resolver))
        .expect("resolved")
        .chart;
    let top = chart.script.as_ref().expect("top-level script");
    assert_eq!(top.src.as_deref(), Some("lib.js"));
    assert_eq!(top.body.as_deref(), Some("function f() { return 1; }"));
    match &chart.states[0].onentry[0][0].kind {
        ExecKind::Script(s) => assert_eq!(s.body.as_deref(), Some("function f() { return 1; }")),
        other => panic!("expected a script, got {other:?}"),
    }
}

// ------------------------------------------------------------------ Vertep Extension Namespace

// @tg: verifies:: [[openspec:scxml-format#Vertep Extension Namespace#vt attribute preserved]] {level: unit}
#[test]
fn vt_attribute_preserved() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0">
  <state id="busy" vt:defer="request.*"/>
  <state id="idle"/>
</scxml>"#;
    let chart = read_str(doc).expect("readable").chart;
    let ext = &chart.states[0].extensions;
    assert_eq!(
        ext.as_slice(),
        [Extension {
            ns: VT_NS.to_owned(),
            local: "defer".to_owned(),
            value: ExtensionValue::Attribute("request.*".to_owned()),
            position: 0,
            span: Default::default(),
        }]
    );
    assert!(chart.states[1].extensions.is_empty());
    let text = write(&chart).expect("writable");
    assert!(
        text.contains(r#"<state id="busy" vt:defer="request.*"/>"#),
        "{text}"
    );
    assert!(text.contains(r#"xmlns:vt="https://w3id.org/vertep/vt#""#));
    assert_eq!(read_str(&text).expect("re-readable").chart, chart);
}

// @tg: verifies:: [[openspec:scxml-format#Vertep Extension Namespace#Other processors unaffected]] {level: unit}
#[test]
fn other_processors_unaffected() {
    let with = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" vt:profile="m0">
  <state id="a" vt:defer="request.*">
    <onentry><vt:trace level="debug"/><log expr="1"/></onentry>
    <transition event="e" target="b" vt:priority="2"/>
  </state>
  <final id="b"/>
</scxml>"#;
    let without = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
  <state id="a">
    <onentry><log expr="1"/></onentry>
    <transition event="e" target="b"/>
  </state>
  <final id="b"/>
</scxml>"#;
    let parsed = read_str(with).expect("vt document");
    assert!(
        parsed.losses.is_empty(),
        "vt: is not a loss: {:?}",
        parsed.losses
    );
    let mut stripped = parsed.chart.clone();
    strip_extensions(&mut stripped);
    assert_ne!(stripped, parsed.chart);
    assert_eq!(stripped, read_str(without).expect("plain document").chart);
    // The written document keeps the SCXML content identical as well.
    let mut written = read_str(&write(&parsed.chart).expect("writable"))
        .expect("re-readable")
        .chart;
    strip_extensions(&mut written);
    assert_eq!(written, stripped);
}

// ------------------------------------------------------------------ Declared Loss Report

const FOREIGN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:acme="urn:acme:tools" version="1.0">
  <state id="a">
    <onentry>
      <log expr="1"/>
      <acme:beep volume="3"/>
    </onentry>
  </state>
</scxml>"#;

// @tg: verifies:: [[openspec:scxml-format#Declared Loss Report#Unknown foreign element listed]] {level: unit}
#[test]
fn unknown_foreign_element_listed() {
    let parsed = read_str(FOREIGN).expect("lenient read");
    let block = &parsed.chart.states[0].onentry[0];
    assert_eq!(block.len(), 2);
    match &block[1].kind {
        ExecKind::Extension(x) => {
            assert_eq!(x.ns, "urn:acme:tools");
            assert_eq!(x.local, "beep");
            assert_eq!(x.position, 1);
            assert_eq!(
                x.value,
                ExtensionValue::Element(
                    r#"<acme:beep xmlns:acme="urn:acme:tools" volume="3"/>"#.to_owned()
                )
            );
        }
        other => panic!("expected an extension node, got {other:?}"),
    }
    assert_eq!(parsed.losses.items.len(), 1);
    let loss = &parsed.losses.items[0];
    assert_eq!(loss.kind, LossKind::ForeignExtension);
    assert!(loss.detail.contains("urn:acme:tools") && loss.detail.contains("beep"));
    assert_eq!((loss.span.line, loss.span.column), (5, 7));
}

// @tg: verifies:: [[openspec:scxml-format#Declared Loss Report#Strict mode rejects loss]] {level: unit}
#[test]
fn strict_mode_rejects_loss() {
    let lenient = read_str(FOREIGN).expect("lenient read").losses;
    match read(FOREIGN, &ReadOptions::new().strict(true)) {
        Err(ReadError::Lossy(report)) => assert_eq!(report, lenient),
        other => panic!("expected a lossy-document error, got {other:?}"),
    }
    // Formatting alone does not fail strict mode.
    let plain = "<!-- c -->\n<scxml xmlns=\"http://www.w3.org/2005/07/scxml\" version=\"1.0\">\n  <state id=\"a\"/>\n</scxml>";
    assert!(read(plain, &ReadOptions::new().strict(true)).is_ok());
}

// ------------------------------------------------------------------ Datamodel Attribute Handling

// @tg: verifies:: [[openspec:scxml-format#Datamodel Attribute Handling#Default datamodel]] {level: unit}
#[test]
fn default_datamodel() {
    let chart = read_str(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0"><state id="a"/></scxml>"#,
    )
    .expect("readable")
    .chart;
    assert_eq!(chart.datamodel, "null");
    assert!(!chart.datamodel_explicit);
    let text = write(&chart).expect("writable");
    assert!(!text.contains("datamodel="), "default not written: {text}");
}

// @tg: verifies:: [[openspec:scxml-format#Datamodel Attribute Handling#Unregistered datamodel]] {level: unit}
#[test]
fn unregistered_datamodel() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" datamodel="xpath"><state id="a"/></scxml>"#;
    let errors = errors_of(read(doc, &ecma()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ErrorKind::UnsupportedDatamodel);
    assert!(errors[0].message.contains("xpath"));
    // Registering it makes the same document readable.
    let chart = read(doc, &ReadOptions::new().with_datamodels(&["xpath"]))
        .expect("registered")
        .chart;
    assert_eq!(chart.datamodel, "xpath");
    assert!(chart.datamodel_explicit);
}
