//! Layout stored inline in a chart file does not change the chart's version IRI: the `vl:`
//! namespace in SCXML and the `layout` block in YAML never reach the IR that is hashed.

use vertep_rdf::{chart_hash, chart_version};

const SCXML_PLAIN: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="idle">
  <state id="idle"><transition event="go" target="done"/></state>
  <final id="done"/>
</scxml>"#;

const SCXML_LAYOUT: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vl="https://w3id.org/vertep/layout#" version="1.0" initial="idle">
  <state id="idle" vl:x="40" vl:y="40" vl:w="120" vl:h="60" vl:collapsed="false">
    <transition event="go" target="done" vl:route="160,70 240,70" vl:label="200,60"/>
  </state>
  <final id="done" vl:x="240" vl:y="50" vl:w="40" vl:h="40"/>
</scxml>"#;

const YAML_PLAIN: &str =
    "vertep: 1\nname: door\nstates:\n  idle:\n    on: { go: done }\n  done: { type: final }\n";

const YAML_LAYOUT: &str = r#"vertep: 1
name: door
states:
  idle:
    on: { go: done }
  done: { type: final }
layout:
  states:
    idle: { x: 40, y: 40, w: 120, h: 60, collapsed: false }
    done: { x: 240, y: 50, w: 40, h: 40 }
  transitions:
    "idle#0": { route: [[160, 70], [240, 70]], label: [200, 60] }
"#;

// @tg: verifies:: [[openspec:scxml-format#Layout Namespace#Layout does not change the chart version]] {level: integration}
#[test]
fn scxml_layout_does_not_change_the_chart_version() {
    let plain = vertep_scxml::read_str(SCXML_PLAIN).expect("plain").chart;
    let laid_out = vertep_scxml::read_str(SCXML_LAYOUT).expect("layout").chart;
    assert_eq!(chart_hash(&laid_out), chart_hash(&plain));
    assert_eq!(chart_version(&laid_out), chart_version(&plain));
}

// @tg: verifies:: [[openspec:yaml-format#Layout Block#Layout does not change the chart version]] {level: integration}
#[test]
fn yaml_layout_does_not_change_the_chart_version() {
    let plain = vertep_yaml::read_str(YAML_PLAIN).unwrap_or_else(|e| panic!("{e}"));
    let laid_out = vertep_yaml::read_str(YAML_LAYOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(chart_hash(&laid_out), chart_hash(&plain));
    assert_eq!(chart_version(&laid_out), chart_version(&plain));
}
