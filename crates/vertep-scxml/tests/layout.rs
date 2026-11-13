//! The layout namespace `https://w3id.org/vertep/layout#` (prefix `vl`) is formatting: it never
//! reaches the IR and never fails strict mode.

use vertep_scxml::{LossKind, ReadOptions, VL_NS, read, read_str};

/// A chart without layout.
const PLAIN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle">
  <state id="idle">
    <transition event="go" target="busy"/>
  </state>
  <parallel id="busy">
    <state id="a"><final id="a_done"/></state>
    <state id="b">
      <history id="b_h" type="deep"/>
      <state id="b1"/>
    </state>
    <transition event="stop" target="idle"/>
  </parallel>
</scxml>
"#;

/// The same chart with the layout a designer writes inline.
const WITH_LAYOUT: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vl="https://w3id.org/vertep/layout#" version="1.0" initial="idle">
  <state id="idle" vl:x="40" vl:y="40" vl:w="120" vl:h="60">
    <transition event="go" target="busy" vl:route="160,70 220,70 220,120" vl:label="190,60"/>
  </state>
  <parallel id="busy" vl:x="220" vl:y="100" vl:w="400" vl:h="300" vl:collapsed="false">
    <state id="a" vl:x="240" vl:y="140" vl:w="160" vl:h="120"><final id="a_done" vl:x="260" vl:y="180" vl:w="40" vl:h="40"/></state>
    <state id="b" vl:x="420" vl:y="140" vl:w="160" vl:h="120" vl:collapsed="true">
      <history id="b_h" type="deep" vl:x="430" vl:y="150" vl:w="20" vl:h="20"/>
      <vl:note text="a designer's sticky note"><vl:anchor at="0,0"/></vl:note>
      <state id="b1" vl:x="460" vl:y="180" vl:w="80" vl:h="40"/>
    </state>
    <transition event="stop" target="idle" vl:route="220,300 40,300 40,100"/>
  </parallel>
</scxml>
"#;

// @tg: verifies:: [[openspec:scxml-format#Layout Namespace#Layout dropped from the IR]] {level: unit}
#[test]
fn layout_dropped_from_the_ir() {
    let plain = read_str(PLAIN).expect("plain chart reads");
    let laid_out = read_str(WITH_LAYOUT).expect("laid-out chart reads");
    assert_eq!(laid_out.chart, plain.chart);
    assert!(laid_out.losses.is_empty(), "{:?}", laid_out.losses.items);
    assert_eq!(laid_out.losses.items, plain.losses.items);

    let layout: Vec<_> = laid_out.losses.of_kind(LossKind::Layout).collect();
    // 33 `vl:` attributes and one `vl:note` element (its child is skipped with it).
    assert_eq!(layout.len(), 34, "{layout:#?}");
    assert!(LossKind::Layout.is_formatting());
    assert!(
        layout
            .iter()
            .all(|l| laid_out.losses.formatting.contains(l))
    );
    assert!(
        layout
            .iter()
            .any(|l| l.detail.contains("vl:route") && l.detail.contains("<transition>"))
    );
    assert!(layout.iter().any(|l| l.detail.contains("vl:note")));

    // Neither the `xmlns:vl` declaration nor any layout node becomes an extension.
    fn no_layout(states: &[vertep_core::ir::State]) {
        for s in states {
            assert!(s.extensions.iter().all(|x| x.ns != VL_NS), "{}", s.id);
            for t in &s.transitions {
                assert!(t.extensions.iter().all(|x| x.ns != VL_NS));
            }
            no_layout(&s.children);
        }
    }
    assert!(laid_out.chart.extensions.is_empty());
    no_layout(&laid_out.chart.states);
}

// @tg: verifies:: [[openspec:scxml-format#Layout Namespace#Strict mode accepts layout]] {level: unit}
#[test]
fn strict_mode_accepts_layout() {
    let strict = ReadOptions::new().strict(true);
    let parsed = read(WITH_LAYOUT, &strict).expect("layout never fails strict mode");
    assert_eq!(parsed.chart, read(PLAIN, &strict).expect("plain").chart);
    assert!(parsed.losses.of_kind(LossKind::Layout).count() > 0);
}

// @tg: verifies:: [[openspec:scxml-format#Layout Namespace#Other foreign namespaces unchanged]] {level: unit}
#[test]
fn other_foreign_namespaces_unchanged() {
    let doc = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vl="https://w3id.org/vertep/layout#" xmlns:acme="urn:acme:tools" version="1.0">
  <state id="idle" vl:x="1" acme:color="red"/>
</scxml>"#;
    let parsed = read_str(doc).expect("lenient read");
    let idle = &parsed.chart.states[0];
    assert_eq!(idle.extensions.len(), 1);
    assert_eq!(idle.extensions[0].ns, "urn:acme:tools");
    assert_eq!(idle.extensions[0].local, "color");
    assert_eq!(parsed.losses.items.len(), 1);
    assert_eq!(parsed.losses.items[0].kind, LossKind::ForeignExtension);
    assert_eq!(parsed.losses.of_kind(LossKind::Layout).count(), 1);
    assert!(read(doc, &ReadOptions::new().strict(true)).is_err());
}

/// The writer never emits layout, so a laid-out document writes exactly like the plain one.
#[test]
fn writer_never_emits_layout() {
    let plain = vertep_scxml::write(&read_str(PLAIN).expect("plain").chart).expect("write");
    let laid_out =
        vertep_scxml::write(&read_str(WITH_LAYOUT).expect("layout").chart).expect("write");
    assert_eq!(laid_out, plain);
    assert!(!laid_out.contains("vl:") && !laid_out.contains(VL_NS));
}
