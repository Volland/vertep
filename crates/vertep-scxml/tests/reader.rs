//! Reader and writer behaviour beyond the spec scenarios: error recovery, namespaces, content,
//! resolution, losses and writer failures.

use std::cell::Cell;

use vertep_core::datamodel::{Datamodel, DatamodelRegistry};
use vertep_core::ir::{
    Chart, Exec, ExecKind, Extension, ExtensionValue, IfBranch, State, StateElement,
};
use vertep_scxml::{
    DeferResolver, ErrorKind, LossKind, MapResolver, ReadOptions, Registered, Resolution,
    SourceKind, SourceRequest, read, read_str, write,
};

const HEAD: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0""#;

fn doc(attrs: &str, body: &str) -> String {
    format!("{HEAD}{attrs}>{body}</scxml>")
}

fn kinds(r: &Result<vertep_scxml::Parsed, vertep_scxml::ReadError>) -> Vec<ErrorKind> {
    match r {
        Ok(_) => Vec::new(),
        Err(e) => e.errors().iter().map(|e| e.kind).collect(),
    }
}

#[test]
fn malformed_xml_is_one_positioned_error() {
    let r = read_str(
        "<scxml xmlns=\"http://www.w3.org/2005/07/scxml\" version=\"1.0\">\n  <state id=\"a\">\n</scxml>",
    );
    let e = r.expect_err("malformed");
    assert_eq!(e.errors().len(), 1, "{e}");
    assert_eq!(e.errors()[0].kind, ErrorKind::Xml);
    assert!(e.errors()[0].span.line >= 2);
}

#[test]
fn unknown_entity_is_an_error() {
    let r = read_str(&doc(
        "",
        "<state id=\"a\"><onentry><assign location=\"x\">&nbsp;</assign></onentry></state>",
    ));
    assert!(kinds(&r).contains(&ErrorKind::Xml));
}

#[test]
fn several_errors_in_one_result_in_document_order() {
    let r = read_str(&doc(
        "",
        r#"
<state id="a">
  <raise event="x"/>
  <frobnicate/>
  <transition target="zzz"/>
</state>
<state id="a"/>"#,
    ));
    let e = r.expect_err("invalid");
    let got: Vec<(ErrorKind, u32)> = e.errors().iter().map(|e| (e.kind, e.span.line)).collect();
    assert_eq!(
        got,
        [
            (ErrorKind::UnexpectedElement, 3),
            (ErrorKind::UnknownElement, 4),
            (ErrorKind::UnknownTarget, 5),
            (ErrorKind::DuplicateStateId, 7),
        ]
    );
}

#[test]
fn root_and_version_checks() {
    assert_eq!(kinds(&read_str("<state/>")), [ErrorKind::NotScxml]);
    assert_eq!(
        kinds(&read_str(
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml"/>"#
        )),
        [ErrorKind::MissingAttribute]
    );
    assert_eq!(
        kinds(&read_str(
            r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="2.0"/>"#
        )),
        [ErrorKind::UnsupportedVersion]
    );
    assert_eq!(kinds(&read_str("")), [ErrorKind::NotScxml]);
}

#[test]
fn invalid_values_and_duplicates() {
    let r = read_str(&doc(
        r#" binding="eager""#,
        r#"<state id="a"><history type="sideways"/><transition type="diagonal"/><initial><transition target="a"/></initial><initial><transition target="a"/></initial></state>"#,
    ));
    let k = kinds(&r);
    assert_eq!(
        k.iter().filter(|k| **k == ErrorKind::InvalidValue).count(),
        3
    );
    assert!(k.contains(&ErrorKind::DuplicateElement));
}

#[test]
fn exclusive_content_combinations() {
    let r = read_str(&doc(
        r#" datamodel="null""#,
        r#"<state id="a"><onentry>
<send event="e" namelist="x"><content>1</content></send>
<assign location="x" expr="1">2</assign>
<cancel/>
</onentry><invoke src="a.scxml"><content>x</content></invoke></state>"#,
    ));
    let k = kinds(&r);
    assert_eq!(
        k.iter()
            .filter(|k| **k == ErrorKind::ExclusiveAttributes)
            .count(),
        3,
        "{k:?}"
    );
    assert!(k.contains(&ErrorKind::MissingAttribute));
}

#[test]
fn prefixed_scxml_and_inherited_namespaces() {
    let text = r#"<s:scxml xmlns:s="http://www.w3.org/2005/07/scxml" xmlns:x="urn:x" version="1.0">
<s:state id="a"><s:onentry><x:hook/></s:onentry></s:state></s:scxml>"#;
    let chart = read_str(text).expect("readable").chart;
    match &chart.states[0].onentry[0][0].kind {
        ExecKind::Extension(x) => {
            assert_eq!(x.ns, "urn:x");
            assert_eq!(
                x.value,
                ExtensionValue::Element(r#"<x:hook xmlns:x="urn:x"/>"#.to_owned())
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn inline_content_text_and_xml() {
    let text = doc(
        r#" datamodel="ecmascript""#,
        r#"<datamodel>
  <data id="t">  a &lt; b  </data>
  <data id="x">
    <item n="1">one</item>
    <item n="2"/>
  </data>
  <data id="w">   </data>
  <data id="amb">&lt;b/&gt;</data>
</datamodel><state id="s"/>"#,
    );
    let parsed = read(&text, &ReadOptions::new().with_datamodels(&["ecmascript"])).expect("ok");
    let d = &parsed.chart.data;
    assert_eq!(d[0].content.as_deref(), Some("  a < b  "));
    assert_eq!(
        d[1].content.as_deref(),
        Some(r#"<item n="1">one</item><item n="2"/>"#)
    );
    assert_eq!(d[2].content, None);
    assert_eq!(d[3].content.as_deref(), Some("<b/>"));
    assert_eq!(parsed.losses.of_kind(LossKind::AmbiguousContent).count(), 1);
}

#[test]
fn attribute_whitespace_round_trips() {
    let text = doc(
        "",
        "<state id=\"a\"><onentry><log expr=\"a&#10;b&#9;c&#13;\" label=\"x\ny\"/></onentry></state>",
    );
    let chart = read_str(&text).expect("ok").chart;
    match &chart.states[0].onentry[0][0].kind {
        ExecKind::Log { label, expr } => {
            assert_eq!(expr.as_deref(), Some("a\nb\tc\r"));
            assert_eq!(
                label.as_deref(),
                Some("x y"),
                "literal newline is normalized"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        read_str(&write(&chart).expect("w")).expect("r").chart,
        chart
    );
}

#[test]
fn crlf_documents() {
    let text = doc(
        r#" datamodel="ecmascript""#,
        "\r\n<state id=\"a\">\r\n<onentry><script>x = 1;\r\ny = 2;</script></onentry>\r\n<transition target=\"b\"/></state><final id=\"b\"/>",
    );
    let chart = read(&text, &ReadOptions::new().with_datamodels(&["ecmascript"]))
        .expect("ok")
        .chart;
    match &chart.states[0].onentry[0][0].kind {
        ExecKind::Script(s) => assert_eq!(s.body.as_deref(), Some("x = 1;\ny = 2;")),
        other => panic!("{other:?}"),
    }
    let t = &chart.states[0].transitions[0];
    assert_eq!((t.span.line, t.span.column), (5, 1));
}

#[test]
fn source_resolution_per_element() {
    let text = doc(
        r#" datamodel="ecmascript""#,
        r#"<datamodel><data id="d" src="d.json"/></datamodel>
<state id="a"><invoke type="scxml" src="child.scxml"/><invoke type="scxml" src="later.scxml"/>
<onentry><send event="e"><content src="payload.txt"/></send></onentry></state>"#,
    );
    let seen = Cell::new(0);
    let resolver = |req: &SourceRequest<'_>| {
        seen.set(seen.get() + 1);
        match (req.kind, req.src) {
            (SourceKind::Data, "d.json") => Ok(Resolution::Inline("[1]".to_owned())),
            (SourceKind::Invoke, "child.scxml") => Ok(Resolution::Inline("<scxml/>".to_owned())),
            (SourceKind::Invoke, _) => Ok(Resolution::Defer),
            (SourceKind::Content, "payload.txt") => Ok(Resolution::Inline("hello".to_owned())),
            _ => Err(vertep_scxml::ResolveError::new("unexpected")),
        }
    };
    let opts = ReadOptions::new()
        .with_datamodels(&["ecmascript"])
        .with_resolver(&resolver);
    let parsed = read(&text, &opts).expect("resolved");
    assert_eq!(seen.get(), 4);
    let c = &parsed.chart;
    assert_eq!(c.data[0].src.as_deref(), Some("d.json"));
    assert_eq!(c.data[0].content.as_deref(), Some("[1]"));
    let inv = &c.states[0].invokes;
    assert_eq!(
        inv[0].content.as_ref().and_then(|c| c.body.as_deref()),
        Some("<scxml/>")
    );
    assert_eq!(inv[1].content, None);
    assert_eq!(parsed.losses.of_kind(LossKind::InlinedSource).count(), 1);
    // Resolved content is written as the reference only, and reads back the same way.
    let written = write(c).expect("writable");
    assert!(!written.contains("[1]") && written.contains(r#"src="d.json""#));
    assert_eq!(read(&written, &opts).expect("re-read").chart.data, c.data);

    // Deferring is not possible for <content src>.
    let defer = DeferResolver;
    let r = read(
        &text,
        &ReadOptions::new()
            .with_datamodels(&["ecmascript"])
            .with_resolver(&defer),
    );
    assert_eq!(kinds(&r), [ErrorKind::UnresolvedSource]);
    // A map resolver fails on unknown entries with an unresolved-source error.
    let map = MapResolver::new().with("d.json", "1");
    let r = read(
        &text,
        &ReadOptions::new()
            .with_datamodels(&["ecmascript"])
            .with_resolver(&map),
    );
    assert_eq!(kinds(&r).len(), 3);
}

#[test]
fn null_datamodel_lists_what_it_cannot_execute() {
    let text = doc(
        "",
        r#"<datamodel><data id="x"/></datamodel><state id="a"><onentry>
<assign location="x" expr="1"/><script>1</script><foreach array="a" item="i"/><log expr="1"/>
</onentry></state>"#,
    );
    let parsed = read_str(&text).expect("ok");
    assert_eq!(
        parsed
            .losses
            .of_kind(LossKind::UnsupportedByDatamodel)
            .count(),
        4
    );
}

#[test]
fn unknown_and_dropped_attributes_are_losses() {
    let text = doc(
        r#" xmlns:x="urn:x" bogus="1""#,
        r#"<state id="a"><onentry x:on="1"><send event="e"><param name="p" expr="1" x:q="2"><x:inner/></param></send></onentry></state>"#,
    );
    let losses = read_str(&text).expect("ok").losses;
    assert_eq!(losses.of_kind(LossKind::UnknownAttribute).count(), 1);
    assert_eq!(losses.of_kind(LossKind::DroppedExtension).count(), 3);
}

#[test]
fn empty_elements_do_not_shift_extension_positions() {
    let text = doc(
        r#" xmlns:vt="https://w3id.org/vertep/vt#" datamodel="ecmascript""#,
        r#"<datamodel/><vt:first/><state id="a"><invoke type="scxml"><finalize/><vt:x/></invoke></state><vt:last/>"#,
    );
    let opts = ReadOptions::new().with_datamodels(&["ecmascript"]);
    let parsed = read(&text, &opts).expect("ok");
    let pos: Vec<u32> = parsed.chart.extensions.iter().map(|x| x.position).collect();
    assert_eq!(pos, [0, 2]);
    assert_eq!(parsed.chart.states[0].invokes[0].extensions[0].position, 0);
    assert_eq!(parsed.losses.of_kind(LossKind::EmptyElement).count(), 2);
    let again = read(&write(&parsed.chart).expect("w"), &opts)
        .expect("r")
        .chart;
    assert_eq!(again, parsed.chart);
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let depth = 300;
    let mut body = String::new();
    for i in 0..depth {
        body.push_str(&format!("<state id=\"s{i}\">"));
    }
    for _ in 0..depth {
        body.push_str("</state>");
    }
    let r = read_str(&doc("", &body));
    assert!(kinds(&r).contains(&ErrorKind::UnexpectedElement));
}

struct OnlyEcma;
struct Dummy;
impl Datamodel for Dummy {
    fn name(&self) -> &str {
        "ecmascript"
    }
    fn init_session(
        &mut self,
        _: &vertep_core::datamodel::SessionInfo,
    ) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
    fn set_environment(&mut self, _: &vertep_core::datamodel::Environment) {}
    fn declare(&mut self, _: &str) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
    fn set_event(&mut self, _: &vertep_core::datamodel::Event) {}
    fn eval_cond(
        &mut self,
        _: &str,
        _: &dyn vertep_core::datamodel::StateQuery,
    ) -> Result<bool, vertep_core::datamodel::EvalError> {
        Ok(true)
    }
    fn eval_value(
        &mut self,
        _: &str,
        _: &dyn vertep_core::datamodel::StateQuery,
    ) -> Result<vertep_core::datamodel::Value, vertep_core::datamodel::EvalError> {
        Ok(vertep_core::datamodel::Value::Null)
    }
    fn eval_content(
        &mut self,
        _: &str,
    ) -> Result<vertep_core::datamodel::Value, vertep_core::datamodel::EvalError> {
        Ok(vertep_core::datamodel::Value::Null)
    }
    fn assign_expr(
        &mut self,
        _: &str,
        _: &str,
        _: &dyn vertep_core::datamodel::StateQuery,
    ) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
    fn assign_value(
        &mut self,
        _: &str,
        _: vertep_core::datamodel::Value,
    ) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
    fn bind_var(
        &mut self,
        _: &str,
        _: vertep_core::datamodel::Value,
    ) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
    fn is_location(&mut self, _: &str) -> bool {
        true
    }
    fn exec_script(
        &mut self,
        _: &str,
        _: &dyn vertep_core::datamodel::StateQuery,
    ) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
    fn foreach_items(
        &mut self,
        _: &str,
        _: &dyn vertep_core::datamodel::StateQuery,
    ) -> Result<Vec<vertep_core::datamodel::Value>, vertep_core::datamodel::EvalError> {
        Ok(Vec::new())
    }
    fn snapshot(&self) -> vertep_core::datamodel::Value {
        vertep_core::datamodel::Value::Null
    }
    fn restore(
        &mut self,
        _: &vertep_core::datamodel::Value,
    ) -> Result<(), vertep_core::datamodel::EvalError> {
        Ok(())
    }
}
impl DatamodelRegistry for OnlyEcma {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        (name == "ecmascript").then(|| Box::new(Dummy) as Box<dyn Datamodel>)
    }
}

#[test]
fn host_registry_decides_registration() {
    let registry = OnlyEcma;
    let set = Registered(&registry);
    let opts = ReadOptions::new().with_datamodels(&set);
    assert!(
        read(
            &doc(r#" datamodel="ecmascript""#, "<state id=\"a\"/>"),
            &opts
        )
        .is_ok()
    );
    assert_eq!(
        kinds(&read(
            &doc(r#" datamodel="cel""#, "<state id=\"a\"/>"),
            &opts
        )),
        [ErrorKind::UnsupportedDatamodel]
    );
    assert!(read(&doc(r#" datamodel="null""#, "<state id=\"a\"/>"), &opts).is_ok());
}

#[test]
fn writer_reports_what_xml_cannot_carry() {
    let mut chart = Chart::default();
    let mut s = State::new("a\u{1}", StateElement::State);
    s.extensions.push(Extension {
        ns: "urn:x".to_owned(),
        local: "not a name".to_owned(),
        value: ExtensionValue::Attribute("v".to_owned()),
        position: 0,
        span: Default::default(),
    });
    s.onentry.push(vec![
        Exec::new(ExecKind::If {
            branches: vec![IfBranch {
                cond: None,
                body: Vec::new(),
                span: Default::default(),
            }],
        }),
        Exec::new(ExecKind::Extension(Extension {
            ns: "urn:x".to_owned(),
            local: "e".to_owned(),
            value: ExtensionValue::Element("<a><b></a>".to_owned()),
            position: 0,
            span: Default::default(),
        })),
    ]);
    chart.states.push(s);
    let err = write(&chart).expect_err("unwritable");
    assert_eq!(err.problems.len(), 4, "{err}");
}
