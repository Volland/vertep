//! A fixed expression corpus whose results and errors are compared with a golden file. The
//! same test runs natively (`cargo test`) and on WebAssembly
//! (`crates/vertep-dm-cel/scripts/wasm-determinism.sh`), so both targets are held to the same
//! bytes. Regenerate the golden file with `CEL_CORPUS_BLESS=1 cargo test -p vertep-dm-cel
//! --test determinism` and review the diff.

use vertep_core::datamodel::{Datamodel, StateQuery, Value};
use vertep_dm_cel::{CelDatamodel, CelValue, Key};

const CORPUS: &str = include_str!("corpus/expressions.txt");
const EXPECTED: &str = include_str!("corpus/expected.txt");

struct OnlyA;

impl StateQuery for OnlyA {
    fn in_state(&self, id: &str) -> bool {
        id == "a"
    }
}

fn render(v: &CelValue) -> String {
    match v {
        CelValue::Null => "null".into(),
        CelValue::Bool(b) => b.to_string(),
        CelValue::Int(i) => i.to_string(),
        CelValue::Uint(u) => format!("{u}u"),
        CelValue::Double(d) => format!("double({d:?}/{:016x})", d.to_bits()),
        CelValue::Decimal(d) => format!("decimal({d})"),
        CelValue::String(s) => format!("{s:?}"),
        CelValue::Bytes(b) => format!("b{b:?}"),
        CelValue::List(l) => format!("[{}]", l.iter().map(render).collect::<Vec<_>>().join(", ")),
        CelValue::Map(m) => format!(
            "{{{}}}",
            m.iter()
                .map(|(k, x)| format!("{}: {}", render(&Key::to_value(k)), render(x)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        CelValue::Timestamp(t) => format!("timestamp({})", t.format()),
        CelValue::Duration(d) => format!("duration({})", d.format()),
        CelValue::Type(t) => format!("type({t})"),
    }
}

fn run_corpus() -> String {
    let mut dm = CelDatamodel::default();
    for (k, src) in [
        ("n", "7"),
        ("d", "decimal('12.50')"),
        ("s", "'héllo'"),
        ("l", "[3, 1, 2]"),
        ("m", "{'b': 2, 'a': 1}"),
        ("f", "0.1"),
    ] {
        dm.declare(k).unwrap();
        dm.assign_expr(k, src, &OnlyA).unwrap();
    }
    let mut out = String::new();
    let mut lines = CORPUS.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        // A line ending inside a triple-quoted string continues on the next line.
        let mut src = line.to_string();
        while src.matches("'''").count() % 2 == 1 {
            match lines.next() {
                Some(next) => {
                    src.push('\n');
                    src.push_str(next);
                }
                None => break,
            }
        }
        let shown = src.replace('\n', "\\n");
        let result = match dm.eval_value(&src, &OnlyA) {
            Ok(v) => {
                let exact = dm
                    .evaluate(&src, &OnlyA)
                    .map(|c| render(&c))
                    .unwrap_or_default();
                format!("ok {exact} | {}", value_text(&v))
            }
            Err(e) => format!("error {}", e.message),
        };
        out.push_str(&format!(
            "{shown}\n  => {result} [steps {}]\n",
            dm.last_steps()
        ));
    }
    out
}

fn value_text(v: &Value) -> String {
    format!("{v:?}")
}

// @tg: verifies:: [[openspec:cel-datamodel#Deterministic Evaluation#Same result on two targets]] {level: integration}
#[test]
fn corpus_matches_the_golden_results() {
    let got = run_corpus();
    #[cfg(not(target_family = "wasm"))]
    if std::env::var_os("CEL_CORPUS_BLESS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus/expected.txt");
        std::fs::write(path, &got).unwrap();
        return;
    }
    for (i, (g, e)) in got.lines().zip(EXPECTED.lines()).enumerate() {
        assert_eq!(
            g,
            e,
            "line {} differs from tests/corpus/expected.txt",
            i + 1
        );
    }
    assert_eq!(got.lines().count(), EXPECTED.lines().count());
    assert_eq!(got, EXPECTED);
}
