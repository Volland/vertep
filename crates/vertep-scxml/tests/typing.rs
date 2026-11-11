//! The SCXML typing extension (`type-system` capability, Typed SCXML Attributes; tasks 3.1
//! and 3.2 of `openspec/changes/m1-typed-authoring`).

use vertep_core::ir::{Block, Chart, ExecKind, Extension, ExtensionValue, State, VT_NS};
use vertep_core::types::chart::{data_type, declarations, is_typed, param_types, send_type};
use vertep_core::types::{BasicTyper, Field, Type, check_chart};
use vertep_scxml::{
    ErrorKind, ReadError, ReadOptions, check_chart_round_trip, check_round_trip, read, write,
};

fn ecma() -> ReadOptions<'static> {
    static DMS: [&str; 2] = ["null", "ecmascript"];
    ReadOptions::new().with_datamodels(&DMS)
}

fn chart(doc: &str) -> Chart {
    read(doc, &ecma()).unwrap_or_else(|e| panic!("{e:?}")).chart
}

const TYPED: &str = r#"<?xml version="1.0"?>
<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
       version="1.0" datamodel="ecmascript" initial="cart">
  <vt:types><![CDATA[type Order = { id: string, total: decimal };
    event pay: { amount: decimal, note: optional<string> };]]></vt:types>
  <datamodel>
    <data id="total" vt:type="decimal" expr="0"/>
    <data id="order" vt:type="optional< Order >"/>
  </datamodel>
  <state id="cart">
    <transition event="checkout" target="done">
      <send event="pay" vt:type="{ amount: decimal, note: optional&lt;string> }">
        <param name="amount" vt:type="decimal" expr="total"/>
        <param name="note" vt:type="optional<string>" expr="'x'"/>
      </send>
    </transition>
  </state>
  <final id="done">
    <donedata><param name="total" vt:type="decimal" expr="total"/></donedata>
  </final>
</scxml>"#;

/// The same document without anything in the `vt:` namespace.
const UNTYPED: &str = r#"<?xml version="1.0"?>
<scxml xmlns="http://www.w3.org/2005/07/scxml"
       version="1.0" datamodel="ecmascript" initial="cart">
  <datamodel>
    <data id="total" expr="0"/>
    <data id="order"/>
  </datamodel>
  <state id="cart">
    <transition event="checkout" target="done">
      <send event="pay">
        <param name="amount" expr="total"/>
        <param name="note" expr="'x'"/>
      </send>
    </transition>
  </state>
  <final id="done">
    <donedata><param name="total" expr="total"/></donedata>
  </final>
</scxml>"#;

fn first_send(c: &Chart) -> &vertep_core::ir::Exec {
    match c.states[0].transitions[0].body.first() {
        Some(
            e @ vertep_core::ir::Exec {
                kind: ExecKind::Send(_),
                ..
            },
        ) => e,
        other => panic!("expected a send, got {other:?}"),
    }
}

// @tg: verifies:: [[openspec:type-system#Typed SCXML Attributes#Typed data element]] {level: unit}
#[test]
fn typed_data_element() {
    let c = chart(
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
                  version="1.0" datamodel="ecmascript">
             <datamodel><data id="total" vt:type="decimal" expr="0"/></datamodel>
             <state id="s"/>
           </scxml>"#,
    );
    assert_eq!(data_type(&c.data[0]), Some(Ok(Type::DECIMAL)));
    let types = check_chart(&c, &BasicTyper::for_datamodel("ecmascript"))
        .unwrap_or_else(|d| panic!("{d:?}"));
    assert!(types.typed);
    assert_eq!(types.context, vec![Field::new("total", Type::DECIMAL)]);
}

#[test]
fn types_are_read_through_the_core_carriers() {
    let c = chart(TYPED);
    // vt:types, in canonical form.
    let decls = declarations(&c).expect("a vt:types block").expect("parses");
    assert_eq!(decls.types.len(), 1);
    assert_eq!(decls.events.len(), 1);
    // <data> and <send>.
    assert_eq!(
        data_type(&c.data[1]),
        Some(Ok(Type::optional(Type::named("Order"))))
    );
    let attr = c.data[1].extensions.iter().find_map(|x| match &x.value {
        ExtensionValue::Attribute(v) if x.ns == VT_NS && x.local == "type" => Some(v.clone()),
        _ => None,
    });
    assert_eq!(attr.as_deref(), Some("optional<Order>"), "canonical text");
    let send = first_send(&c);
    assert_eq!(
        send_type(send),
        Some(Ok(Type::record([
            ("amount", Type::DECIMAL),
            ("note", Type::optional(Type::STRING))
        ])))
    );
    // <param>s of <send> and <donedata> go to their owners' vt:paramTypes.
    assert_eq!(
        param_types(&send.extensions),
        Some(Ok(vec![
            Field::new("amount", Type::DECIMAL),
            Field::new("note", Type::optional(Type::STRING)),
        ]))
    );
    let done = &c.states[1];
    assert_eq!(
        param_types(&done.extensions),
        Some(Ok(vec![Field::new("total", Type::DECIMAL)]))
    );
    // The chart type-checks.
    check_chart(&c, &BasicTyper::for_datamodel("ecmascript")).unwrap_or_else(|d| panic!("{d:?}"));
}

#[test]
fn typed_chart_round_trips_with_vt_type_on_params() {
    let c = check_round_trip(TYPED, &ecma()).unwrap_or_else(|e| panic!("{e}"));
    let written = write(&c).unwrap();
    assert!(
        written.contains(r#"<param name="amount" expr="total" vt:type="decimal"/>"#),
        "{written}"
    );
    assert!(
        written.contains(r#"<param name="total" expr="total" vt:type="decimal"/>"#),
        "{written}"
    );
    assert!(!written.contains("vt:paramTypes"), "{written}");
    assert!(written.contains(r#"<data id="total" expr="0" vt:type="decimal"/>"#));
    // A record that does not match the params stays on the owner and still round-trips.
    let mut odd = c.clone();
    let send = match &mut odd.states[0].transitions[0].body[0].kind {
        ExecKind::Send(s) => s,
        _ => unreachable!(),
    };
    send.params.reverse();
    let w = check_chart_round_trip(&odd, &ecma()).unwrap_or_else(|e| panic!("{e}"));
    assert!(w.contains("vt:paramTypes="), "{w}");
}

#[test]
fn equivalent_spellings_read_to_the_same_chart() {
    let spaced = TYPED
        .replace(
            "{ amount: decimal, note: optional&lt;string> }\">",
            "{amount:decimal,note:optional&lt;string&gt;}\">",
        )
        .replace(
            "<![CDATA[type Order = { id: string, total: decimal };\n    event pay: { amount: decimal, note: optional<string> };]]>",
            "type Order={id:string,total:decimal}; event pay:{amount:decimal,note:optional&lt;string>};",
        );
    assert_ne!(spaced, TYPED);
    assert_eq!(chart(&spaced), chart(TYPED));
}

fn strip_vt(c: &mut Chart) {
    fn exts(v: &mut Vec<Extension>) {
        v.retain(|x| x.ns != VT_NS);
    }
    fn block(b: &mut Block) {
        for e in b.iter_mut() {
            exts(&mut e.extensions);
            match &mut e.kind {
                ExecKind::If { branches } => branches.iter_mut().for_each(|br| block(&mut br.body)),
                ExecKind::Foreach { body, .. } => block(body),
                _ => {}
            }
        }
    }
    fn state(s: &mut State) {
        exts(&mut s.extensions);
        s.data.iter_mut().for_each(|d| exts(&mut d.extensions));
        s.onentry
            .iter_mut()
            .chain(s.onexit.iter_mut())
            .for_each(block);
        s.transitions.iter_mut().for_each(|t| block(&mut t.body));
        s.invokes.iter_mut().for_each(|i| exts(&mut i.extensions));
        s.children.iter_mut().for_each(state);
    }
    exts(&mut c.extensions);
    c.data.iter_mut().for_each(|d| exts(&mut d.extensions));
    c.states.iter_mut().for_each(state);
}

// @tg: verifies:: [[openspec:type-system#Typed SCXML Attributes#Foreign processor ignores types]] {level: unit}
#[test]
fn foreign_processor_ignores_types() {
    // The typed document is well-formed XML: a plain XML parser without namespace awareness
    // reads it to the end.
    let mut r = quick_xml::Reader::from_str(TYPED);
    loop {
        match r.read_event() {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(_) => {}
            Err(e) => panic!("not well-formed: {e}"),
        }
    }
    // It reads as SCXML without any content loss.
    let typed = read(TYPED, &ecma()).expect("valid SCXML");
    assert!(typed.losses.is_empty(), "{:?}", typed.losses);
    assert!(is_typed(&typed.chart));
    // Ignoring the vt: namespace leaves exactly the untyped chart: the extensions are the only
    // difference, and the untyped chart is untyped.
    let mut ignored = typed.chart.clone();
    strip_vt(&mut ignored);
    let untyped = chart(UNTYPED);
    assert_eq!(ignored, untyped);
    assert!(!is_typed(&untyped));
    assert_ne!(typed.chart, untyped);
}

// @tg: verifies:: [[openspec:type-system#Typed SCXML Attributes#Unknown type name]] {level: unit}
#[test]
fn unknown_type_name() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0">
  <vt:types>type Order = { id: string };</vt:types>
  <datamodel>
    <data id="order" vt:type="Ordr"/>
  </datamodel>
  <state id="s"/>
</scxml>"#;
    let errors = match read(doc, &ecma()) {
        Err(ReadError::Invalid { errors, .. }) => errors,
        other => panic!("expected a type error, got {other:?}"),
    };
    assert_eq!(errors.len(), 1, "{errors:?}");
    let e = &errors[0];
    assert_eq!(e.kind, ErrorKind::Type);
    assert!(e.message.contains("`Ordr`"), "{}", e.message);
    assert!(e.message.contains("did you mean `Order`?"), "{}", e.message);
    assert_eq!((e.span.line, e.span.column), (4, 5));
}

#[test]
fn type_errors_in_params_and_blocks_are_located() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#" version="1.0" datamodel="ecmascript">
  <vt:types>type Payment = Card | Cash; type Bad = { x: Pament };</vt:types>
  <state id="s">
    <onentry>
      <send event="e"><param name="p" vt:type="list&lt;Paymnt>" expr="1"/></send>
      <send event="f" vt:type="{ broken"/>
    </onentry>
  </state>
</scxml>"#;
    let errors = match read(doc, &ecma()) {
        Err(ReadError::Invalid { errors, .. }) => errors,
        other => panic!("expected type errors, got {other:?}"),
    };
    let msgs: Vec<String> = errors
        .iter()
        .map(|e| format!("{}:{} {}", e.span.line, e.span.column, e.message))
        .collect();
    assert_eq!(errors.len(), 3, "{msgs:#?}");
    assert!(errors.iter().all(|e| e.kind == ErrorKind::Type));
    assert!(
        msgs[0].starts_with("2:3 ") && msgs[0].contains("did you mean `Payment`?"),
        "{msgs:#?}"
    );
    assert!(
        msgs[1].starts_with("5:23 ") && msgs[1].contains("`Paymnt`"),
        "{msgs:#?}"
    );
    assert!(
        msgs[2].starts_with("6:7 ") && msgs[2].contains("does not parse"),
        "{msgs:#?}"
    );
}
