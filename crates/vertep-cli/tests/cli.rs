//! Integration tests of the `vt` binary: one per command-line scenario of the `type-system`
//! (Command Line Type Tools) and `yaml-format` (Command Line Conversion) capabilities.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value as Json;

const VT: &str = env!("CARGO_BIN_EXE_vt");

/// The design's `order` chart (design.md section 8), CEL datamodel.
const ORDER_YAML: &str = r#"vertep: 1
name: order
datamodel: cel
types:
  Pay: { amount: decimal }
  Approval: { by: iri }
context:
  total: { type: decimal, init: "0" }
  limit: { type: decimal, init: "100" }
events: { start: null, pay: Pay, authorized: null, approve: Approval, cancel: null }
initial: idle
states:
  idle:
    on: { start: checkout }
  checkout:
    initial: cart
    on: { cancel: cancelled }
    states:
      cart:
        on:
          pay:
            - cond: "ev.data.amount > 0"
              target: payment
              actions: [ { assign: { location: ctx.total, expr: "ev.data.amount" } } ]
      payment:
        on:
          authorized:
            - { cond: "ctx.total > ctx.limit", target: review }
            - { target: paid }
      review:
        on: { approve: paid }
  paid: { type: final }
  cancelled: { type: final }
"#;

/// The same chart written by hand in SCXML: different layout, attribute order, quoting and
/// type spelling.
const ORDER_SCXML: &str = r#"<?xml version="1.0"?>
<!-- the order chart -->
<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:vt="https://w3id.org/vertep/vt#"
       initial="idle" datamodel="cel" version="1.0" name="order">
  <vt:types><![CDATA[
    type Pay = {amount: decimal};
    type Approval = { by : iri };
    event start; event pay: Pay; event authorized; event approve: Approval; event cancel;
  ]]></vt:types>
  <datamodel>
    <data vt:type="decimal" id="total" expr="0"/>
    <data id="limit" expr="100" vt:type=" decimal "/>
  </datamodel>
  <state id="idle"><transition event="start" target="checkout"/></state>
  <state id="checkout" initial="cart">
    <transition event="cancel" target="cancelled"/>
    <state id="cart">
      <transition event="pay" cond="ev.data.amount > 0" target="payment">
        <assign location="ctx.total" expr="ev.data.amount"/>
      </transition>
    </state>
    <state id="payment">
      <transition event="authorized" cond="ctx.total &gt; ctx.limit" target="review"/>
      <transition event="authorized" target="paid"/>
    </state>
    <state id="review"><transition event="approve" target="paid"/></state>
  </state>
  <final id="paid"/>
  <final id="cancelled"/>
</scxml>
"#;

struct Dir(tempfile::TempDir);

impl Dir {
    fn new() -> Self {
        Dir(tempfile::tempdir().expect("temp dir"))
    }

    fn file(&self, name: &str, content: &str) -> PathBuf {
        let p = self.0.path().join(name);
        std::fs::write(&p, content).expect("write fixture");
        p
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.path().join(name)
    }
}

fn vt(args: &[&str]) -> Output {
    Command::new(VT)
        .args(args)
        .env_remove("VT_CERTIFY")
        .output()
        .expect("vt runs")
}

fn p(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

// ------------------------------------------------------------ Command Line Type Tools

// @tg: verifies:: [[openspec:type-system#Command Line Type Tools#Typecheck failure exit code]] {level: integration}
#[test]
fn typecheck_failure_exit_code() {
    let d = Dir::new();
    let bad = ORDER_YAML
        .replace("ev.data.amount > 0", "ctx.limitt > 0")
        .replace("ctx.total > ctx.limit", "ctx.missing > 1");
    let f = d.file("order.yaml", &bad);
    let o = vt(&["typecheck", p(&f)]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let err = stderr(&o);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 2, "{err}");
    assert!(lines[0].starts_with(&format!("{}:22:15:", p(&f))), "{err}");
    assert!(lines[0].contains("`limitt`"), "{err}");
    assert!(lines[1].starts_with(&format!("{}:28:15:", p(&f))), "{err}");
    assert!(lines[1].contains("`missing`"), "{err}");
    assert!(stdout(&o).is_empty());

    // The same in SCXML: two type errors with line and column.
    let bad_scxml = ORDER_SCXML
        .replace(
            r#"<data id="limit" expr="100" vt:type=" decimal "/>"#,
            r#"<data id="limit" expr="'x'" vt:type="decimal"/>"#,
        )
        .replace("ctx.total &gt; ctx.limit", "ctx.missing &gt; 1");
    let f = d.file("order.scxml", &bad_scxml);
    let o = vt(&["typecheck", p(&f)]);
    assert_eq!(o.status.code(), Some(1));
    let err = stderr(&o);
    assert!(err.contains(&format!("{}:12:5:", p(&f))), "{err}");
    assert!(err.contains(&format!("{}:23:7:", p(&f))), "{err}");
    assert!(err.contains("2 type errors"), "{err}");

    // A good chart passes.
    let f = d.file("good.yaml", ORDER_YAML);
    let o = vt(&["typecheck", p(&f)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).contains("ok: 2 named types, 5 typed events, 2 context fields"));
}

// @tg: verifies:: [[openspec:type-system#Command Line Type Tools#Schema export]] {level: integration}
#[test]
fn schema_export() {
    let d = Dir::new();
    let f = d.file("order.scxml", ORDER_SCXML);
    let o = vt(&["schema", p(&f), "--format", "jsonschema"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let doc: Json = serde_json::from_str(&stdout(&o)).expect("JSON");
    let dialect = Json::String("https://json-schema.org/draft/2020-12/schema".into());
    // The context type.
    let ctx = &doc["context"];
    assert_eq!(ctx["$schema"], dialect);
    assert_eq!(ctx["type"], "object");
    assert_eq!(ctx["required"], serde_json::json!(["total", "limit"]));
    // One document per declared event.
    let events = doc["events"].as_object().expect("events");
    let names: Vec<&str> = events.keys().map(String::as_str).collect();
    assert_eq!(names, ["approve", "authorized", "cancel", "pay", "start"]);
    for (name, s) in events {
        assert_eq!(s["$schema"], dialect, "{name}");
    }
    assert_eq!(events["pay"]["$ref"], "#/$defs/Pay");
    assert_eq!(
        events["pay"]["$defs"]["Pay"]["properties"]["amount"]["anyOf"][0]["type"],
        "number"
    );
    assert_eq!(events["start"]["type"], "null");

    // The other projections.
    let o = vt(&["schema", p(&f), "--format", "shacl"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let ttl = stdout(&o);
    assert!(ttl.contains("urn:vertep:type:sha256:"), "{ttl}");
    assert!(ttl.contains("sh:NodeShape"), "{ttl}");
    let o = vt(&["schema", p(&f), "--format", "cel"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).contains("ctx.total: decimal"), "{}", stdout(&o));
    let o = vt(&["schema", p(&f), "--format", "rust"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("vertep-macros"));
}

// @tg: verifies:: [[openspec:type-system#Command Line Type Tools#Hash is surface independent]] {level: integration}
#[test]
fn hash_is_surface_independent() {
    let d = Dir::new();
    let y = d.file("order.yaml", ORDER_YAML);
    let s = d.file("order.scxml", ORDER_SCXML);
    let hy = vt(&["hash", p(&y)]);
    let hs = vt(&["hash", p(&s)]);
    assert_eq!(hy.status.code(), Some(0), "{}", stderr(&hy));
    assert_eq!(hs.status.code(), Some(0), "{}", stderr(&hs));
    let iri = stdout(&hy);
    assert!(iri.starts_with("urn:vertep:chart:sha256:"), "{iri}");
    assert_eq!(iri.trim().len(), "urn:vertep:chart:sha256:".len() + 64);
    assert_eq!(stdout(&hs), iri);
    // And through RDF.
    let ttl = d.path("order.ttl");
    let c = vt(&["convert", p(&s), "--to", "turtle", "-o", p(&ttl)]);
    assert_eq!(c.status.code(), Some(0), "{}", stderr(&c));
    assert_eq!(stdout(&vt(&["hash", p(&ttl)])), iri);
    // A different type is a different chart.
    let other = d.file("other.yaml", &ORDER_YAML.replace("by: iri", "by: string"));
    assert_ne!(stdout(&vt(&["hash", p(&other)])), iri);
}

// @tg: verifies:: [[openspec:type-system#Command Line Type Tools#Certify without the Lean checker]] {level: integration}
#[test]
fn certify_without_the_lean_checker() {
    let d = Dir::new();
    let f = d.file("order.yaml", ORDER_YAML);
    let empty = d.path("empty-bin");
    std::fs::create_dir(&empty).unwrap();
    let o = Command::new(VT)
        .args(["certify", p(&f)])
        .env_remove("VT_CERTIFY")
        .env("PATH", &empty)
        .output()
        .unwrap();
    assert_ne!(o.status.code(), Some(0));
    assert!(stdout(&o).is_empty(), "no certificate: {}", stdout(&o));
    let err = stderr(&o);
    assert!(err.contains("not installed"), "{err}");
    assert!(err.contains("lake build"), "{err}");
    assert!(err.contains("VT_CERTIFY"), "{err}");
}

#[cfg(unix)]
#[test]
fn certify_runs_the_lean_checker() {
    use std::os::unix::fs::PermissionsExt;
    let d = Dir::new();
    let f = d.file("order.yaml", ORDER_YAML);
    // A stand-in with the Lean checker's interface (`vt-certify --chart FILE`): certifies the
    // chart whose `#chart` node the file holds.
    let checker = d.file(
        "vt-certify",
        "#!/bin/sh\n[ \"$1\" = --chart ] || exit 3\nv=$(grep -o '<urn:vertep:chart:sha256:[0-9a-f]*#chart>' \"$2\" | head -1 | sed 's/#chart>//; s/^<//')\n[ -n \"$v\" ] || { echo rejected >&2; exit 2; }\necho \"<$v> <https://w3id.org/vertep/vt#certifiedBy> <urn:vertep:cert:test> .\"\n",
    );
    std::fs::set_permissions(&checker, std::fs::Permissions::from_mode(0o755)).unwrap();
    let o = Command::new(VT)
        .args(["certify", p(&f)])
        .env("VT_CERTIFY", &checker)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let version = stdout(&vt(&["hash", p(&f)])).trim().to_owned();
    assert_eq!(
        stdout(&o).trim(),
        format!("<{version}> <https://w3id.org/vertep/vt#certifiedBy> <urn:vertep:cert:test> .")
    );
}

/// The Lean checker of `lean/vertep-lean`, when it is built.
fn lean_checker() -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lean/vertep-lean/.lake/build/bin")
        .join("vt-certify");
    p.is_file().then_some(p)
}

/// Typed charts certify with the real Lean checker: the YAML and SCXML `order` charts and every
/// chart under `crates/*/tests/charts/`. The checker certifies only when its chart version,
/// which covers the type triples it derives from the chart's `vt:types` block, is the version
/// `vt hash` prints.
// @tg: verifies:: [[openspec:formal-model#Chart Certificates#Typed chart certified]] {level: e2e}
#[test]
fn certify_typed_charts_with_the_lean_checker() {
    let Some(checker) = lean_checker() else {
        eprintln!("skipped: lean/vertep-lean is not built (lake build vt-certify)");
        return;
    };
    let d = Dir::new();
    let mut files = vec![
        d.file("order.yaml", ORDER_YAML),
        d.file("order.scxml", ORDER_SCXML),
    ];
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for krate in std::fs::read_dir(&crates).expect("crates") {
        let Ok(charts) = std::fs::read_dir(krate.expect("crate").path().join("tests/charts")) else {
            continue;
        };
        for f in charts {
            let path = f.expect("chart").path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("scxml" | "yaml")
            ) {
                files.push(path);
            }
        }
    }
    assert!(files.len() > 2, "no chart under crates/*/tests/charts");
    for f in &files {
        let o = Command::new(VT)
            .args(["certify", p(f)])
            .env("VT_CERTIFY", &checker)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(0), "{}: {}", f.display(), stderr(&o));
        let version = stdout(&vt(&["hash", p(f)])).trim().to_owned();
        assert!(
            stdout(&o).starts_with(&format!(
                "<{version}> <https://w3id.org/vertep/vt#certifiedBy> <urn:vertep:cert:sha256:"
            )),
            "{}: {}",
            f.display(),
            stdout(&o)
        );
    }
}

#[test]
fn validate_reports_well_formedness_and_conformance() {
    let d = Dir::new();
    let f = d.file("order.yaml", ORDER_YAML);
    let o = vt(&["validate", p(&f)]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    assert!(stdout(&o).contains("ok: urn:vertep:chart:sha256:"));
    // A final state with a transition is not well-formed.
    let bad = d.file(
        "bad.scxml",
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0">
  <final id="f"><transition event="e" target="f"/></final>
</scxml>"#,
    );
    let o = vt(&["validate", p(&bad)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!stderr(&o).is_empty());
}

// ------------------------------------------------------------- Command Line Conversion

// @tg: verifies:: [[openspec:yaml-format#Command Line Conversion#YAML to SCXML]] {level: integration}
#[test]
fn yaml_to_scxml() {
    let d = Dir::new();
    let y = d.file("order.yaml", ORDER_YAML);
    let o = vt(&["convert", p(&y), "--to", "scxml"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let scxml = stdout(&o);
    assert!(
        scxml.contains(r#"<data id="total" expr="0" vt:type="decimal"/>"#),
        "{scxml}"
    );
    assert!(scxml.contains("<vt:types"), "{scxml}");
    assert!(scxml.contains("type Pay = { amount: decimal };"), "{scxml}");
    // Its IR equals the YAML chart's IR.
    let from_yaml = vertep_yaml::read_str(ORDER_YAML).expect("yaml");
    let from_scxml = vertep_scxml::read(
        &scxml,
        &vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel),
    )
    .expect("scxml")
    .chart;
    assert_eq!(from_scxml, from_yaml);
    // And back to YAML byte for byte.
    let s = d.file("order.scxml", &scxml);
    let back = vt(&["convert", p(&s), "--to", "yaml"]);
    assert_eq!(back.status.code(), Some(0), "{}", stderr(&back));
    assert_eq!(stdout(&back), vertep_yaml::write(&from_yaml).unwrap());
}

// @tg: verifies:: [[openspec:yaml-format#Command Line Conversion#Conversion to RDF]] {level: integration}
#[test]
fn conversion_to_rdf() {
    let d = Dir::new();
    let s = d.file("order.scxml", ORDER_SCXML);
    let o = vt(&["convert", p(&s), "--to", "turtle"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let ttl = stdout(&o);
    let back = vertep_rdf::read_chart(ttl.as_bytes(), vertep_rdf::RdfFormat::Turtle)
        .expect("the output is the chart's RDF 1.2 projection in Turtle");
    let chart = vertep_scxml::read(
        ORDER_SCXML,
        &vertep_scxml::ReadOptions::new().with_datamodels(&vertep_scxml::AnyDatamodel),
    )
    .unwrap()
    .chart;
    assert_eq!(back.chart, chart);
    assert!(ttl.contains("vt:declaresType"), "types are in the graph");
    for (to, check) in [("trig", "{"), ("nquads", "> .")] {
        let o = vt(&["convert", p(&s), "--to", to]);
        assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
        assert!(stdout(&o).contains(check), "{to}");
    }
}

// @tg: verifies:: [[openspec:yaml-format#Command Line Conversion#Lossy conversion refused]] {level: integration}
#[test]
fn lossy_conversion_refused() {
    let d = Dir::new();
    let s = d.file(
        "lossy.scxml",
        r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a" color="red">
    <onentry foo:bar="1" xmlns:foo="urn:foo"><log label="x"/></onentry>
  </state>
</scxml>"#,
    );
    let out = d.path("lossy.yaml");
    let o = vt(&["convert", p(&s), "--to", "yaml", "-o", p(&out)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stdout(&o).is_empty());
    assert!(!out.exists(), "no output is written");
    let err = stderr(&o);
    assert!(err.contains("would lose"), "{err}");
    assert!(err.contains("attribute `color`"), "{err}");
    assert!(err.contains("{urn:foo}bar"), "{err}");
    assert_eq!(
        err.lines().filter(|l| l.starts_with("  - ")).count(),
        2,
        "{err}"
    );
    // Comments and layout are not information: the hand-written chart converts.
    let f = d.file("order.scxml", ORDER_SCXML);
    assert_eq!(
        vt(&["convert", p(&f), "--to", "yaml"]).status.code(),
        Some(0)
    );
}

#[test]
fn usage_and_io_errors_exit_2() {
    let o = vt(&["hash", "/nonexistent/chart.yaml"]);
    assert_eq!(o.status.code(), Some(2));
    let o = vt(&["convert", "x.yaml", "--to", "png"]);
    assert_eq!(o.status.code(), Some(2));
}

// @tg: verifies:: [[openspec:type-system#Types Survive Every Surface#YAML to SCXML to RDF and back]] {level: integration}
#[test]
fn yaml_to_scxml_to_rdf_and_back() {
    let d = Dir::new();
    let y = d.file("order.yaml", ORDER_YAML);
    let s = d.path("order.scxml");
    let t = d.path("order.ttl");
    let y2 = d.path("back.yaml");
    for (from, to, out) in [(&y, "scxml", &s), (&s, "turtle", &t), (&t, "yaml", &y2)] {
        let o = vt(&["convert", p(from), "--to", to, "-o", p(out)]);
        assert_eq!(o.status.code(), Some(0), "{to}: {}", stderr(&o));
    }
    assert_eq!(
        stdout(&vt(&["hash", p(&y2)])),
        stdout(&vt(&["hash", p(&y)]))
    );
    let back = vertep_yaml::read_str(&std::fs::read_to_string(&y2).unwrap()).unwrap();
    let orig = vertep_yaml::read_str(ORDER_YAML).unwrap();
    assert_eq!(
        vertep_core::types::chart::declarations(&back),
        vertep_core::types::chart::declarations(&orig)
    );
    assert_eq!(back, orig);
}
