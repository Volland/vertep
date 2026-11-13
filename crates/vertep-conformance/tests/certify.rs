//! The Rust validator and the Lean `WellFormed` check (`vt-certify`) agree on generated charts:
//! well-formed ones from `vertep-testkit`, and the same charts broken by one targeted mutation
//! per validation rule.
//!
//! The full sample is `#[ignore]` (`--features differential -- --ignored differential`;
//! `VT_CERTIFY_CHARTS` sets its size); a small sample runs with the gate. Without the Lean
//! build both skip with a message, except under the `differential` feature.

use vertep_conformance::LeanTools;
use vertep_conformance::differential::{
    Certification, MUTATIONS, certify, sample_validation_charts,
};
use vertep_core::ir::Chart;
use vertep_core::validate::validate;

const REQUIRED: bool = cfg!(feature = "differential");

/// Compare the verdicts on `n` sampled charts; returns the disagreements and the rejections
/// `vt-certify` gave as "not a chart projection" (exit 2). A dangling reference (an unknown
/// target) cannot be carried by the projection, so Lean rejects such charts at decoding
/// rather than in `WellFormed`; that still counts as the same verdict when Rust rejects too.
fn disagreements(tools: &LeanTools, n: usize) -> (Vec<String>, Vec<String>) {
    let charts = sample_validation_charts(n, 0x6365_7274);
    let workers = std::thread::available_parallelism()
        .map(|w| w.get())
        .unwrap_or(4)
        .min(16);
    let chunks: Vec<&[(Chart, Option<usize>)]> = charts
        .chunks(charts.len().div_ceil(workers).max(1))
        .collect();
    let results: Vec<(Vec<String>, Vec<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    let mut undecodable = Vec::new();
                    for (chart, mutation) in chunk.iter() {
                        let rust = validate(chart);
                        let what = mutation.map_or("well-formed sample", |m| MUTATIONS[m]);
                        let lean = match certify(tools, chart) {
                            Ok(c) => c,
                            Err(e) => {
                                out.push(format!("{what}: {e}"));
                                continue;
                            }
                        };
                        let agree = match (&rust, &lean) {
                            (Ok(()), Certification::Certified) => true,
                            (Err(_), Certification::NotWellFormed(_)) => true,
                            (Err(_), Certification::NotAProjection(m)) => {
                                undecodable.push(format!("{what}: {m}"));
                                true
                            }
                            _ => false,
                        };
                        if !agree {
                            out.push(format!(
                                "{what}: Rust {:?}, Lean {lean:?}\n{}",
                                rust.err(),
                                vertep_scxml::write(chart).unwrap_or_default()
                            ));
                        }
                    }
                    (out, undecodable)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("certify worker"))
            .collect()
    });
    let mut bad = Vec::new();
    let mut undecodable = Vec::new();
    for (b, u) in results {
        bad.extend(b);
        undecodable.extend(u);
    }
    (bad, undecodable)
}

fn check(n: usize) {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let (bad, undecodable) = disagreements(&tools, n);
    println!(
        "validation: {n} charts ({} mutated), {} disagreements, {} rejected by both with Lean at decoding",
        n / 2,
        bad.len(),
        undecodable.len()
    );
    for u in &undecodable {
        println!("  {}", u.lines().next().unwrap_or(""));
    }
    assert!(
        bad.is_empty(),
        "Rust validate and vt-certify disagree:\n{}",
        bad.join("\n---\n")
    );
}

// @tg: verifies:: [[openspec:formal-model#IR Model In Lean#Same verdict as Rust validation]] {level: integration}
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_validation_matches_vt_certify() {
    let n = std::env::var("VT_CERTIFY_CHARTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(192);
    check(n);
}

#[test]
fn conformance_validation_smoke_sample_matches_vt_certify() {
    // Every mutation once, plus as many well-formed charts.
    check(2 * MUTATIONS.len());
}

// ------------------------------------------------------------------------------ typed charts

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

use vertep_core::ir::{Extension, ExtensionValue, Span, VT_NS};
use vertep_core::types::{Declarations, EventDecl, Field, Primitive, Type, TypeDecl, Variant};

const TYPE_NAMES: [&str; 5] = ["A", "B", "C", "Order_2", "Z"];
const FIELD_NAMES: [&str; 6] = ["a", "b", "a b", "\u{fc}ber", "x\"y", "tag"];
const TAGS: [&str; 6] = ["X", "Y", "list", "with space", "\u{3bb}", "value"];

fn gen_type(r: &mut Rng, depth: usize) -> Type {
    if depth == 0 || r.chance(35) {
        return match r.below(8) {
            0..=4 => Type::Primitive(*r.pick(&Primitive::ALL)),
            5 => Type::Value,
            _ => Type::named(*r.pick(&TYPE_NAMES)),
        };
    }
    match r.below(4) {
        0 => Type::list(gen_type(r, depth - 1)),
        1 => Type::optional(gen_type(r, depth - 1)),
        2 => Type::Record(
            (0..r.below(4))
                .map(|_| Field::new(*r.pick(&FIELD_NAMES), gen_type(r, depth - 1)))
                .collect(),
        ),
        _ => Type::Union(
            (0..1 + r.below(3))
                .map(|_| {
                    let p = r.chance(60).then(|| gen_type(r, depth - 1));
                    Variant::new(*r.pick(&TAGS), p)
                })
                .collect(),
        ),
    }
}

/// A declaration block: random types (forward references, unknown names, duplicates, groups
/// of mutually recursive types) and events.
fn gen_decls(r: &mut Rng) -> Declarations {
    let n = 1 + r.below(4);
    let mut types: Vec<TypeDecl> = (0..n)
        .map(|_| TypeDecl {
            name: (*r.pick(&TYPE_NAMES[..4])).to_owned(),
            ty: gen_type(r, 3),
            span: Span::NONE,
        })
        .collect();
    if r.chance(25) {
        // A recursive group through a list and an optional.
        types.push(TypeDecl {
            name: "A".into(),
            ty: Type::record([("next", Type::optional(Type::named("B")))]),
            span: Span::NONE,
        });
        types.push(TypeDecl {
            name: "B".into(),
            ty: Type::list(Type::named("A")),
            span: Span::NONE,
        });
    }
    let events = (0..r.below(3))
        .map(|i| EventDecl {
            name: [format!("e{i}"), format!("ev.{i}"), format!("e v{i}")][r.below(3)].clone(),
            payload: r.chance(50).then(|| gen_type(r, 2)),
            span: Span::NONE,
        })
        .collect();
    Declarations { types, events }
}

fn escape_text(text: &str, r: &mut Rng) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' if r.chance(50) => out.push_str("&gt;"),
            '"' if r.chance(30) => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            ';' if r.chance(10) => out.push_str("&#59;"),
            'e' if r.chance(10) => out.push_str("&#x65;"),
            c => out.push(c),
        }
    }
    out
}

/// The text of a block in a random layout: comments, blank lines and tabs between
/// declarations, sometimes corrupted by a random edit.
fn gen_text(r: &mut Rng, d: &Declarations) -> String {
    let mut text = String::new();
    for line in d.to_text().lines() {
        if r.chance(15) {
            text.push_str("# a comment: type X = ;\n");
        }
        if r.chance(10) {
            text.push_str("\t \n");
        }
        text.push_str(line);
        text.push(if r.chance(10) { ' ' } else { '\n' });
    }
    if r.chance(20) {
        let edits = [
            '|', ';', '{', '}', '"', '\\', '#', ':', ',', 'x', '\u{b}', '\u{a0}', '\n',
        ];
        let chars: Vec<char> = text.chars().collect();
        if !chars.is_empty() {
            let at = r.below(chars.len());
            let mut out: String = chars[..at].iter().collect();
            if r.chance(50) {
                out.push(*r.pick(&edits));
                out.extend(&chars[at..]);
            } else {
                out.extend(&chars[at + 1..]);
            }
            text = out;
        }
    }
    text
}

/// The serialized `vt:types` element around a text: escaped, as CDATA, or broken XML.
fn gen_element(r: &mut Rng, text: &str) -> String {
    let open = match r.below(3) {
        0 => format!("<vt:types xmlns:vt=\"{VT_NS}\">"),
        1 => format!("<vt:types a='x>y' xmlns:vt=\"{VT_NS}\">"),
        _ => "<vt:types>".to_owned(),
    };
    let ws = if r.chance(20) { "\n  " } else { "" };
    match r.below(10) {
        0..=5 => format!("{ws}{open}{}</vt:types>{ws}", escape_text(text, r)),
        6..=7 if !text.contains("]]>") => {
            format!("{open}<![CDATA[{text}]]>{}</vt:types>", escape_text(" ", r))
        }
        8 => match r.below(5) {
            0 => format!("{open}&bogus;{}</vt:types>", escape_text(text, r)),
            1 => format!("{open}<![CDATA[{text}</vt:types>"),
            2 => format!("{open}<b/>{}</vt:types>", escape_text(text, r)),
            3 => "<vt:types/>".to_owned(),
            _ => format!("{open}{}", escape_text(text, r)),
        },
        _ => format!("{open}&#x20;{}&#32;</vt:types>", escape_text(text, r)),
    }
}

/// A well-formed chart carrying `xml` as its `vt:types` element.
fn typed_chart(xml: String) -> Chart {
    let src = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" version="1.0" initial="a">
  <state id="a"><transition event="go" target="b"/></state>
  <final id="b"/>
</scxml>"#;
    let mut chart = vertep_scxml::read_str(src).expect("base chart").chart;
    chart.extensions.insert(
        0,
        Extension {
            ns: VT_NS.to_owned(),
            local: "types".to_owned(),
            value: ExtensionValue::Element(xml),
            position: 0,
            span: Span::NONE,
        },
    );
    chart
}

/// Certify `n` generated typed charts; every one is well-formed, so `vt-certify` must certify
/// each, which it does only when its chart version (covering the type triples it derives from
/// the `vt:types` block) equals the Rust version the node IRIs are minted under. Returns the
/// failures and how many charts had type triples and how many blocks did not parse.
fn typed_disagreements(tools: &LeanTools, n: usize, seed: u64) -> (Vec<String>, usize, usize) {
    let mut r = Rng(seed);
    let (mut bad, mut typed, mut unparsed) = (Vec::new(), 0, 0);
    for _ in 0..n {
        let d = gen_decls(&mut r);
        let text = gen_text(&mut r, &d);
        let xml = gen_element(&mut r, &text);
        let chart = typed_chart(xml.clone());
        match vertep_core::types::chart::declarations(&chart) {
            Some(Ok(decls)) => {
                typed += usize::from(!vertep_rdf::TypeIris::of(&decls).is_empty());
            }
            _ => unparsed += 1,
        }
        match certify(tools, &chart) {
            Ok(Certification::Certified) => {}
            other => bad.push(format!("{other:?}\n{xml}")),
        }
    }
    (bad, typed, unparsed)
}

fn check_typed(n: usize) {
    let Some(tools) = LeanTools::for_test(REQUIRED) else {
        return;
    };
    let (bad, typed, unparsed) = typed_disagreements(&tools, n, 0x7479_7065_6463_6572);
    println!("typed charts: {n}, {typed} with type triples, {unparsed} blocks that do not parse");
    assert!(
        bad.is_empty(),
        "vt-certify refused typed charts ({} of {n}):\n{}",
        bad.len(),
        bad.join("\n---\n")
    );
    assert!(typed * 2 > n, "only {typed} of {n} charts had type triples");
    assert!(unparsed > 0, "no block failed to parse");
}

// @tg: verifies:: [[openspec:formal-model#Chart Certificates#Typed chart certified]] {level: integration}
#[test]
#[ignore = "needs the Lean build; run with --features differential -- --ignored differential"]
fn differential_typed_charts_certify() {
    let n = std::env::var("VT_CERTIFY_TYPED_CHARTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    check_typed(n);
}

#[test]
fn conformance_typed_charts_smoke_sample_certify() {
    check_typed(60);
}
