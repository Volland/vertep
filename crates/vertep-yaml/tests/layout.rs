//! The optional top-level `layout` block is a designer's visual layout: the reader accepts any
//! value there and ignores it, and the writer never emits it.

use vertep_yaml::{read_str, write};

const PLAIN: &str = r#"vertep: 1
name: light
states:
  green:
    on: { next: yellow }
  yellow:
    on: { next: red }
  red:
    type: final
"#;

const WITH_LAYOUT: &str = r#"vertep: 1
name: light
states:
  green:
    on: { next: yellow }
  yellow:
    on: { next: red }
  red:
    type: final
layout:
  states:
    green: { x: 40, y: 40, w: 120, h: 60 }
    yellow: { x: 200, y: 40, w: 120, h: 60, collapsed: false }
    red: { x: 360, y: 40, w: 40, h: 40 }
  transitions:
    "green#0": { route: [[160, 70], [200, 70]], label: [180, 60] }
    "yellow#0": { route: [[320, 70], [360, 60]], label: [340, 55] }
"#;

// @tg: verifies:: [[openspec:yaml-format#Layout Block#Layout ignored by the reader]] {level: unit}
#[test]
fn layout_ignored_by_the_reader() {
    let plain = read_str(PLAIN).unwrap_or_else(|e| panic!("{e}"));
    let laid_out = read_str(WITH_LAYOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(laid_out, plain);
    assert!(laid_out.extensions.is_empty());

    // Any YAML value is accepted.
    for value in ["null", "42", "[1, 2]", "{ anything: { goes: [here] } }"] {
        let doc = format!("{PLAIN}layout: {value}\n");
        assert_eq!(read_str(&doc).unwrap_or_else(|e| panic!("{e}")), plain);
    }
}

// @tg: verifies:: [[openspec:yaml-format#Layout Block#Layout never written]] {level: unit}
#[test]
fn layout_never_written() {
    let laid_out = read_str(WITH_LAYOUT).unwrap_or_else(|e| panic!("{e}"));
    let written = write(&laid_out).expect("write");
    assert_eq!(
        written,
        write(&read_str(PLAIN).expect("plain")).expect("write")
    );
    assert!(!written.contains("layout"));
}

/// A misspelt top-level key is still a schema error.
#[test]
fn other_unknown_keys_still_rejected() {
    let doc = format!("{PLAIN}layuot: {{}}\n");
    let err = read_str(&doc).expect_err("unknown key");
    assert!(err.to_string().contains("layuot"), "{err}");
}
