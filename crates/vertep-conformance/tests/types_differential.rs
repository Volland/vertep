//! The value checker against the Lean model (verification-backlog 4.5): generated declaration
//! blocks, types and values are checked by `Declarations::check`, and `vertep-types` (the Lean
//! `check`, proven equivalent to the least-fixed-point typing relation `HasType`) must give the
//! same verdict on every case.
//!
//! Values are generated from the type with random perturbations, so both verdicts occur often,
//! and the blocks include names recursing only through `optional` (decision "Type checking
//! stops on non-contractive recursion"), unknown names and alias cycles. Numbers stay away from
//! the edges of the signed 64-bit range, where the Rust `f64` and the Lean exact decimal of its
//! JSON text differ by construction.
//!
//! Without the Lean build the test skips with a message, except under the `differential`
//! feature, where a missing build fails.

use std::path::{Path, PathBuf};

use vertep_conformance::lean::lean_bin_dir;
use vertep_core::datamodel::Value;
use vertep_core::ir::Span;
use vertep_core::types::{Declarations, Field, Primitive, Type, TypeDecl, Variant};

const REQUIRED: bool = cfg!(feature = "differential");

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

const NAMES: [&str; 4] = ["A", "B", "C", "Z"];
const FIELDS: [&str; 3] = ["a", "b", "c"];
const TAGS: [&str; 3] = ["X", "Y", "W"];
const NUMBERS: [f64; 10] = [0.0, 1.0, -7.0, 42.0, 1.5, 2.5, -0.25, 1e10, 9.5e18, -1e300];
const STRINGS: [&str; 22] = [
    "",
    "x",
    "true",
    "12.50",
    "12,50",
    "-0.10",
    "1.",
    "2026-11-11T10:00:00Z",
    "2026-11-11T10:00:00",
    "2026-02-30T10:00:00Z",
    "2024-02-29t23:59:60.5+02:00",
    "2026-11-11T10:00:00+24:00",
    "https://example.org/a",
    "urn:uuid:1",
    "orders/42",
    "https://ex.org/a b",
    "http://ex.org/%zz",
    "https://ex.org/\u{fc}",
    "tag",
    "X",
    "Y",
    "value",
];
const PRIMS: [Primitive; 6] = Primitive::ALL;

fn gen_type(r: &mut Rng, depth: usize) -> Type {
    let leaf = depth == 0 || r.chance(35);
    if leaf {
        return match r.below(8) {
            0..=4 => Type::Primitive(*r.pick(&PRIMS)),
            5 => Type::Value,
            _ => Type::named(*r.pick(&NAMES)),
        };
    }
    match r.below(4) {
        0 => Type::list(gen_type(r, depth - 1)),
        1 => Type::optional(gen_type(r, depth - 1)),
        2 => {
            let n = r.below(4);
            Type::Record(
                (0..n)
                    .map(|_| Field::new(*r.pick(&FIELDS), gen_type(r, depth - 1)))
                    .collect(),
            )
        }
        _ => {
            let n = 1 + r.below(3);
            Type::Union(
                (0..n)
                    .map(|_| {
                        let p = r.chance(60).then(|| gen_type(r, depth - 1));
                        Variant::new(*r.pick(&TAGS), p)
                    })
                    .collect(),
            )
        }
    }
}

fn decl(name: &str, ty: Type) -> TypeDecl {
    TypeDecl {
        name: name.into(),
        ty,
        span: Span::default(),
    }
}

/// A block: random definitions of `A`, `B`, `C`, or one of the recursion shapes.
fn gen_decls(r: &mut Rng) -> Declarations {
    let types = match r.below(6) {
        0 => vec![
            decl("A", Type::optional(Type::named("A"))),
            decl("B", Type::optional(Type::named("C"))),
            decl("C", Type::optional(Type::named("B"))),
        ],
        1 => vec![
            decl("A", Type::optional(Type::optional(Type::named("A")))),
            decl(
                "B",
                Type::record([("a", Type::named("A")), ("n", Type::INT)]),
            ),
            decl("C", Type::named("B")),
        ],
        2 => vec![
            decl(
                "A",
                Type::record([("l", Type::STRING), ("c", Type::list(Type::named("A")))]),
            ),
            decl("B", Type::named("C")),
            decl("C", Type::named("B")),
        ],
        _ => NAMES[..3].iter().map(|n| decl(n, gen_type(r, 3))).collect(),
    };
    Declarations {
        types,
        events: Vec::new(),
    }
}

fn gen_any(r: &mut Rng, depth: usize) -> Value {
    match r.below(if depth == 0 { 5 } else { 7 }) {
        0 => Value::Undefined,
        1 => Value::Null,
        2 => Value::Bool(r.chance(50)),
        3 => Value::Number(*r.pick(&NUMBERS)),
        4 => Value::String(r.pick(&STRINGS).to_string()),
        5 => Value::Array((0..r.below(3)).map(|_| gen_any(r, depth - 1)).collect()),
        _ => Value::Object(
            (0..r.below(3))
                .map(|_| {
                    let k = *r.pick(&["a", "b", "tag", "value", "q"]);
                    (k.to_string(), gen_any(r, depth - 1))
                })
                .collect(),
        ),
    }
}

/// A value shaped by `ty`, with perturbations.
fn gen_value(r: &mut Rng, d: &Declarations, ty: &Type, depth: usize) -> Value {
    if depth == 0 || r.chance(12) {
        return gen_any(r, 2);
    }
    match ty {
        Type::Value => gen_any(r, 2),
        Type::Named(n) => match d.lookup(n) {
            Some(t) => gen_value(r, d, t, depth - 1),
            None => gen_any(r, 1),
        },
        Type::Primitive(Primitive::Bool) => Value::Bool(r.chance(50)),
        Type::Primitive(Primitive::Int | Primitive::Decimal) if r.chance(60) => {
            Value::Number(*r.pick(&NUMBERS))
        }
        Type::Primitive(_) => Value::String(r.pick(&STRINGS).to_string()),
        Type::Optional(t) => match r.below(10) {
            0..=2 => Value::Null,
            3 => Value::Undefined,
            _ => gen_value(r, d, t, depth - 1),
        },
        Type::List(t) => Value::Array(
            (0..r.below(4))
                .map(|_| gen_value(r, d, t, depth - 1))
                .collect(),
        ),
        Type::Record(fields) => {
            let mut out = Vec::new();
            for f in fields {
                match r.below(10) {
                    0 => {}
                    1 => out.push((f.name.clone(), Value::Undefined)),
                    _ => out.push((f.name.clone(), gen_value(r, d, &f.ty, depth - 1))),
                }
            }
            if r.chance(10) {
                out.push(("q".into(), gen_any(r, 1)));
            }
            Value::Object(out)
        }
        Type::Union(vs) => {
            let v = r.pick(vs);
            let mut out = vec![("tag".to_string(), Value::String(v.tag.clone()))];
            if r.chance(10) {
                out[0].1 = Value::String(r.pick(&STRINGS).to_string());
            }
            match &v.payload {
                Some(p) if r.chance(85) => {
                    out.push(("value".into(), gen_value(r, d, p, depth - 1)));
                }
                _ if r.chance(10) => out.push(("value".into(), gen_any(r, 1))),
                _ => {}
            }
            if r.chance(5) {
                out.push(("q".into(), Value::Null));
            }
            Value::Object(out)
        }
    }
}

fn types_exe() -> Option<PathBuf> {
    let exe = lean_bin_dir().join("vertep-types");
    if Path::new(&exe).is_file() {
        Some(exe)
    } else if REQUIRED {
        panic!(
            "{} is not built: run `cd lean/vertep-lean && lake build vertep-types`",
            exe.display()
        )
    } else {
        eprintln!("skipped: {} is not built", exe.display());
        None
    }
}

/// Write the cases and ask `vertep-types`; `Err` with its report on a disagreement.
fn judge(exe: &Path, cases: &[serde_json::Value]) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("vertep-types-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("cases.jsonl");
    let text: Vec<String> = cases.iter().map(|c| c.to_string()).collect();
    std::fs::write(&path, text.join("\n")).map_err(|e| e.to_string())?;
    let out = std::process::Command::new(exe)
        .arg("--cases")
        .arg(&path)
        .output()
        .map_err(|e| format!("{}: {e}", exe.display()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = out?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

fn case(d: &Declarations, ty: &Type, v: &Value) -> serde_json::Value {
    serde_json::json!({
        "decls": d,
        "ty": ty,
        "value": v,
        "ok": d.check(ty, v).is_ok(),
    })
}

// @tg: verifies:: [[openspec:formal-model#Type Checker Correctness#Checker decides the typing relation]] {level: integration}
#[test]
fn differential_type_checker_agrees_with_lean() {
    let Some(exe) = types_exe() else { return };
    let mut r = Rng(0x5eed_7e57_0000_0001);
    let mut cases = Vec::new();
    let (mut accepted, mut validated) = (0, 0);
    for _ in 0..4000 {
        let d = gen_decls(&mut r);
        let ty = if r.chance(50) {
            Type::named(*r.pick(&NAMES))
        } else {
            gen_type(&mut r, 3)
        };
        let v = gen_value(&mut r, &d, &ty, 6);
        let c = case(&d, &ty, &v);
        accepted += usize::from(c["ok"] == serde_json::Value::Bool(true));
        validated += usize::from(d.validate().is_empty());
        cases.push(c);
    }
    // Enough of both verdicts, and of blocks `validate` accepts, to mean something.
    assert!(
        accepted > 500 && accepted < 3500,
        "accepted {accepted} of 4000"
    );
    assert!(validated > 1000, "validated blocks {validated} of 4000");
    match judge(&exe, &cases) {
        Ok(msg) => println!("{msg}; Rust accepted {accepted}, {validated} blocks validate"),
        Err(e) => panic!("the Rust value checker disagrees with the Lean model: {e}"),
    }
}

/// Recursion through `optional` only, against the Lean model (the Rust unit test
/// `recursion_through_optional_only_terminates`).
// @tg: verifies:: [[openspec:formal-model#Type Checker Correctness#Recursion through optional only terminates]] {level: integration}
#[test]
fn differential_type_checker_recursion_through_optional() {
    let Some(exe) = types_exe() else { return };
    let d = Declarations {
        types: vec![
            decl("A", Type::optional(Type::named("A"))),
            decl("B", Type::optional(Type::named("C"))),
            decl("C", Type::optional(Type::named("B"))),
            decl("D", Type::optional(Type::optional(Type::named("D")))),
            decl(
                "R",
                Type::record([("a", Type::named("A")), ("n", Type::INT)]),
            ),
        ],
        events: Vec::new(),
    };
    let mut cases = Vec::new();
    for n in ["A", "B", "D"] {
        for v in [
            Value::Null,
            Value::String("x".into()),
            Value::Object(Vec::new()),
        ] {
            cases.push(case(&d, &Type::named(n), &v));
        }
    }
    let rec = Value::Object(vec![
        ("a".into(), Value::Bool(true)),
        ("n".into(), Value::Number(1.0)),
    ]);
    cases.push(case(&d, &Type::named("R"), &rec));
    assert_eq!(
        cases.iter().filter(|c| c["ok"] == true).count(),
        3,
        "only null is a value"
    );
    if let Err(e) = judge(&exe, &cases) {
        panic!("the Rust value checker disagrees with the Lean model: {e}");
    }
}
