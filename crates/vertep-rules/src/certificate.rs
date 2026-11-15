//! Derivation certificates ([[openspec:rule-verification#Derivation Certificate]]): a rule
//! decision of the certifiable fragment as Lean source that the kernel checks against
//! `VertepLean.RuleCert`, run as a process in the way `vt certify` runs the chart checker.
//!
//! The fragment is clauses `tm.conclude (triple S P O) [Vars] :- atoms` whose atoms are
//! `tm.triple` calls and integer comparisons (`=<`, `<`, `>=`, `>`, `=`). A program outside it
//! has no certificate (negation, other predicates), which is reported, never guessed. The Rust
//! side finds the instance by matching the clause against the bundled statements; the Lean side
//! does not trust that matcher, only checks the instance it is given. What stays trusted is the
//! clause parser here and the eid check of the host (see the module comment of `RuleCert.lean`).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use crate::{RuleBundle, RulesError, Term, clauses};

#[derive(Clone, Debug, PartialEq)]
enum Pat {
    Var(String),
    Num(String),
    Term(Term),
}

#[derive(Clone, Debug, PartialEq)]
enum Atom {
    Triple(Pat, Pat, Pat, String),
    Cmp(&'static str, Pat, Pat),
}

#[derive(Clone, Debug)]
struct ClauseShape {
    head: (Pat, Pat, Pat),
    support: Vec<String>,
    body: Vec<Atom>,
}

fn outside(why: impl Into<String>) -> RulesError {
    RulesError::Program(format!("outside the certifiable fragment: {}", why.into()))
}

/// Splits at commas that are not inside brackets or strings.
fn split_top(s: &str, sep: char) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut in_str) = (Vec::new(), String::new(), 0i32, false);
    for c in s.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            '(' | '[' if !in_str => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' if !in_str => {
                depth -= 1;
                cur.push(c);
            }
            c if c == sep && depth == 0 && !in_str => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter().map(|x| x.trim().to_owned()).collect()
}

/// The top-level whitespace-separated items of an atom.
fn items(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut in_str) = (Vec::new(), String::new(), 0i32, false);
    for c in s.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                cur.push(c);
            }
            '(' | '[' if !in_str => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' if !in_str => {
                depth -= 1;
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 && !in_str => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn unquote(s: &str) -> Option<String> {
    s.strip_prefix('"')?.strip_suffix('"').map(str::to_owned)
}

fn is_var(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_uppercase() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn pat(s: &str, fresh: &mut usize) -> Result<Pat, RulesError> {
    let s = s.trim();
    if is_var(s) {
        return Ok(if s == "_" {
            *fresh += 1;
            Pat::Var(format!("_{fresh}"))
        } else {
            Pat::Var(s.to_owned())
        });
    }
    if let Some(inner) = s.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        let parts = items(inner);
        if let [kind, arg] = parts.as_slice() {
            match (kind.as_str(), unquote(arg)) {
                ("iri", Some(x)) => return Ok(Pat::Term(Term::Iri(x))),
                ("str", Some(x)) => return Ok(Pat::Term(Term::Str(x))),
                ("num", None) if is_var(arg) => return Ok(Pat::Num(arg.clone())),
                ("num", None) => {
                    if let Ok(n) = arg.parse::<i64>() {
                        return Ok(Pat::Term(Term::Int(n)));
                    }
                }
                _ => {}
            }
        }
    }
    if let Ok(n) = s.parse::<i64>() {
        return Ok(Pat::Term(Term::Int(n)));
    }
    Err(outside(format!("the term `{s}`")))
}

fn parse_clause(text: &str) -> Result<ClauseShape, RulesError> {
    let text = text.trim().trim_end_matches('.');
    let (head, body) = text
        .split_once(":-")
        .ok_or_else(|| outside("a fact, not a rule"))?;
    let head = head.trim();
    let rest = head
        .strip_prefix("tm.conclude")
        .ok_or_else(|| outside("the head is not tm.conclude"))?
        .trim();
    let parts = items(rest);
    let [fact, sup] = parts.as_slice() else {
        return Err(outside("the head shape"));
    };
    let triple = fact
        .strip_prefix('(')
        .and_then(|x| x.strip_suffix(')'))
        .and_then(|x| x.trim().strip_prefix("triple"))
        .ok_or_else(|| outside("the conclusion is not a triple"))?;
    let ts = items(triple);
    let mut fresh = 0;
    let [hs, hp, ho] = ts.as_slice() else {
        return Err(outside("the conclusion triple"));
    };
    let head = (
        pat(hs, &mut fresh)?,
        pat(hp, &mut fresh)?,
        pat(ho, &mut fresh)?,
    );
    let support = sup
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .map(|x| split_top(x, ','))
        .ok_or_else(|| outside("the support list"))?;
    if support.iter().any(|v| !is_var(v)) {
        return Err(outside("support entries must be variables"));
    }
    let mut atoms = Vec::new();
    for a in split_top(body, ',') {
        let it = items(&a);
        if it.first().map(String::as_str) == Some("tm.triple") {
            let [_, s, p, o, e] = it.as_slice() else {
                return Err(outside(format!("the atom `{a}`")));
            };
            if !is_var(e) {
                return Err(outside("the statement id of tm.triple must be a variable"));
            }
            atoms.push(Atom::Triple(
                pat(s, &mut fresh)?,
                pat(p, &mut fresh)?,
                pat(o, &mut fresh)?,
                e.clone(),
            ));
        } else if let [l, op, r] = it.as_slice() {
            let op = match op.as_str() {
                "=<" => "le",
                "<" => "lt",
                ">=" => "ge",
                ">" => "gt",
                "=" => "eq",
                _ => return Err(outside(format!("the atom `{a}`"))),
            };
            atoms.push(Atom::Cmp(op, pat(l, &mut fresh)?, pat(r, &mut fresh)?));
        } else {
            return Err(outside(format!("the atom `{a}`")));
        }
    }
    Ok(ClauseShape {
        head,
        support,
        body: atoms,
    })
}

type Binding = BTreeMap<String, Term>;

fn inst(p: &Pat, b: &Binding) -> Option<Term> {
    match p {
        Pat::Var(v) => b.get(v).cloned(),
        Pat::Num(v) => match b.get(v) {
            Some(Term::Int(n)) => Some(Term::Int(*n)),
            _ => None,
        },
        Pat::Term(t) => Some(t.clone()),
    }
}

/// Unifies a pattern with a fact term, extending the binding.
fn unify(p: &Pat, t: &Term, b: &mut Binding) -> bool {
    match p {
        Pat::Term(x) => x == t,
        Pat::Var(v) => match b.get(v) {
            Some(x) => x == t,
            None => {
                b.insert(v.clone(), t.clone());
                true
            }
        },
        Pat::Num(v) => {
            if !matches!(t, Term::Int(_)) {
                return false;
            }
            match b.get(v) {
                Some(x) => x == t,
                None => {
                    b.insert(v.clone(), t.clone());
                    true
                }
            }
        }
    }
}

fn cmp_holds(op: &str, l: &Term, r: &Term) -> bool {
    let (Term::Int(x), Term::Int(y)) = (l, r) else {
        return false;
    };
    match op {
        "le" => x <= y,
        "lt" => x < y,
        "ge" => x >= y,
        "gt" => x > y,
        _ => x == y,
    }
}

type Tape<'a> = &'a [(Term, Term, Term, String)];

/// Finds a binding under which every atom holds against the tape and the head and the support
/// are the recorded ones.
fn search(
    c: &ClauseShape,
    i: usize,
    tape: Tape<'_>,
    b: &Binding,
    want: &(Term, Term, Term),
    want_support: &[String],
) -> Option<Binding> {
    let Some(atom) = c.body.get(i) else {
        let h = (
            inst(&c.head.0, b)?,
            inst(&c.head.1, b)?,
            inst(&c.head.2, b)?,
        );
        let ids: Option<Vec<String>> = c
            .support
            .iter()
            .map(|v| match b.get(v) {
                Some(Term::Str(id)) => Some(id.clone()),
                _ => None,
            })
            .collect();
        let mut got = ids?;
        got.sort();
        let mut w = want_support.to_vec();
        w.sort();
        return (h == *want && got == w).then(|| b.clone());
    };
    match atom {
        Atom::Triple(s, p, o, e) => tape.iter().find_map(|(fs, fp, fo, id)| {
            let mut nb = b.clone();
            (unify(s, fs, &mut nb)
                && unify(p, fp, &mut nb)
                && unify(o, fo, &mut nb)
                && unify(&Pat::Var(e.clone()), &Term::Str(id.clone()), &mut nb))
            .then(|| search(c, i + 1, tape, &nb, want, want_support))
            .flatten()
        }),
        Atom::Cmp(op, l, r) => {
            let (l, r) = (inst(l, b)?, inst(r, b)?);
            cmp_holds(op, &l, &r)
                .then(|| search(c, i + 1, tape, b, want, want_support))
                .flatten()
        }
    }
}

fn lean_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn lean_term(t: &Term) -> Result<String, RulesError> {
    Ok(match t {
        Term::Iri(s) => format!(".iri {}", lean_str(s)),
        Term::Str(s) => format!(".str {}", lean_str(s)),
        Term::Int(n) => format!(".int ({n})"),
        other => return Err(outside(format!("the term {other:?}"))),
    })
}

fn lean_pat(p: &Pat) -> Result<String, RulesError> {
    Ok(match p {
        Pat::Var(v) => format!(".var {}", lean_str(v)),
        Pat::Num(v) => format!(".num {}", lean_str(v)),
        Pat::Term(t) => format!(".term ({})", lean_term(t)?),
    })
}

/// Lean source whose theorem `cert` states that the bundle's decision is a derivation of its
/// clause from the bundled statements. `cert` is proved by `decide`, so the kernel does the
/// checking; the file ends with `#print axioms cert`.
// @tg: implements:: [[openspec:rule-verification#Derivation Certificate]] {confidence: partial, note: "the certifiable fragment: tm.triple atoms and integer comparisons"}
pub fn lean_certificate(bundle: &RuleBundle) -> Result<String, RulesError> {
    let d = &bundle.derivation;
    if d.engine != crate::ENGINE {
        return Err(RulesError::EngineMismatch {
            recorded: d.engine.clone(),
            running: crate::ENGINE.to_owned(),
        });
    }
    let tape: Vec<(Term, Term, Term, String)> = bundle
        .statements
        .iter()
        .map(|s| (s.s.clone(), s.p.clone(), s.o.clone(), s.eid.clone()))
        .collect();
    let want = (
        Term::Iri(d.node.clone()),
        Term::Iri(crate::vocab::DECISION.to_owned()),
        Term::Iri(format!("{}{}", crate::vocab::EVENT_PREFIX, d.event)),
    );
    let mut reasons = Vec::new();
    for prog in &bundle.programs {
        for text in clauses(&prog.text) {
            let c = match parse_clause(&text) {
                Ok(c) => c,
                Err(e) => {
                    if text.starts_with("tm.conclude") {
                        reasons.push(e.to_string());
                    }
                    continue;
                }
            };
            let Some(b) = search(&c, 0, &tape, &Binding::new(), &want, &d.support) else {
                continue;
            };
            let ids: Vec<String> = c
                .support
                .iter()
                .map(|v| match b.get(v) {
                    Some(Term::Str(id)) => id.clone(),
                    _ => String::new(),
                })
                .collect();
            let mut out = format!(
                "-- engine: {}\n-- closure: {}\nimport VertepLean.RuleCert\nopen Vertep.RuleCert\n\n",
                d.engine, d.closure_digest
            );
            out.push_str("def tape : List Fact := [\n");
            for (i, (s, p, o, id)) in tape.iter().enumerate() {
                let (Ok(s), Ok(p), Ok(o)) = (lean_term(s), lean_term(p), lean_term(o)) else {
                    continue;
                };
                out.push_str(&format!(
                    "  {{ s := {s}, p := {p}, o := {o}, eid := {} }}{}\n",
                    lean_str(id),
                    if i + 1 < tape.len() { "," } else { "" }
                ));
            }
            out.push_str("]\n\n");
            out.push_str(&format!(
                "def clause : Clause := {{\n  headS := {}, headP := {}, headO := {},\n  support := [{}],\n  body := [\n",
                lean_pat(&c.head.0)?,
                lean_pat(&c.head.1)?,
                lean_pat(&c.head.2)?,
                c.support.iter().map(|v| lean_str(v)).collect::<Vec<_>>().join(", ")
            ));
            for (i, a) in c.body.iter().enumerate() {
                let line = match a {
                    Atom::Triple(s, p, o, e) => format!(
                        ".triple ({}) ({}) ({}) {}",
                        lean_pat(s)?,
                        lean_pat(p)?,
                        lean_pat(o)?,
                        lean_str(e)
                    ),
                    Atom::Cmp(op, l, r) => {
                        format!(".cmp .{op} ({}) ({})", lean_pat(l)?, lean_pat(r)?)
                    }
                };
                out.push_str(&format!(
                    "    {line}{}\n",
                    if i + 1 < c.body.len() { "," } else { "" }
                ));
            }
            out.push_str("  ] }\n\n");
            let binding: Vec<String> = b
                .iter()
                .map(|(k, v)| Ok(format!("({}, {})", lean_str(k), lean_term(v)?)))
                .collect::<Result<_, RulesError>>()?;
            out.push_str(&format!(
                "def binding : Binding := [{}]\n\n",
                binding.join(", ")
            ));
            out.push_str(&format!(
                "theorem cert : check tape clause binding ({}) ({}) ({}) [{}] = true := by decide\n\n#print axioms cert\n",
                lean_term(&want.0)?,
                lean_term(&want.1)?,
                lean_term(&want.2)?,
                ids.iter().map(|i| lean_str(i)).collect::<Vec<_>>().join(", ")
            ));
            return Ok(out);
        }
    }
    Err(if reasons.is_empty() {
        RulesError::Diverged(
            "no clause of the bundled program derives this decision from the bundled statements"
                .to_owned(),
        )
    } else {
        RulesError::Program(reasons.join("; "))
    })
}

/// What the Lean kernel said about a certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertReport {
    /// The axioms the theorem depends on, as Lean prints them.
    pub axioms: String,
}

/// Checks Lean source `source` in the `vertep-lean` package at `lean_dir` by running
/// `lake env lean` as a process. The kernel must accept `cert`, and the theorem may depend only on
/// `propext`, `Classical.choice` and `Quot.sound`.
// @tg: implements:: [[openspec:rule-verification#Derivation Certificate]] {confidence: partial, note: "runs the checker as a process, as vt certify does"}
pub fn check_certificate(lean_dir: &Path, source: &str) -> Result<CertReport, RulesError> {
    let dir = std::env::temp_dir().join(format!("vertep-rule-cert-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| RulesError::Store(e.to_string()))?;
    let file = dir.join("Cert.lean");
    std::fs::write(&file, source).map_err(|e| RulesError::Store(e.to_string()))?;
    let lake = std::env::var("VT_LAKE").unwrap_or_else(|_| "lake".to_owned());
    let out = Command::new(lake)
        .args(["env", "lean"])
        .arg(&file)
        .current_dir(lean_dir)
        .output()
        .map_err(|e| RulesError::Store(format!("cannot run lake: {e}")))?;
    let _ = std::fs::remove_dir_all(&dir);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() || text.contains("error") {
        return Err(RulesError::Diverged(format!(
            "the Lean kernel rejected the certificate: {}",
            text.lines().take(4).collect::<Vec<_>>().join(" | ")
        )));
    }
    let axioms = text
        .lines()
        .find(|l| l.contains("depends on axioms") || l.contains("does not depend on any axioms"))
        .unwrap_or("")
        .to_owned();
    if axioms.contains("sorryAx") || axioms.contains("native") {
        return Err(RulesError::Diverged(format!(
            "unacceptable axioms: {axioms}"
        )));
    }
    Ok(CertReport { axioms })
}
