//! The rules profile: the subset of the rule language for which totality, support necessity and
//! certificates are defined ([[openspec:rule-verification#Rules Profile]]).
//!
//! A program outside the profile still runs; its decisions are marked unchecked. The check is
//! lexical and on predicate names: it does not parse the language, so it is conservative about
//! what it accepts (a construct it cannot see is not flagged) and exact about what it flags.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Host builtins a profile program may call.
pub const HOST_BUILTINS: &[&str] = &[
    "tm.triple",
    "tm.conclude",
    "tm.violation",
    "tm.violation-delta",
    "tm.added",
    "tm.retracted",
    "tm.valid",
];

/// Tokens that put a clause outside the profile, with the reason.
const FORBIDDEN: &[(&str, &str)] = &[
    ("findall", "findall depends on the order of solutions"),
    ("print", "input and output are effects"),
    ("read", "input and output are effects"),
    ("halt", "control of the host is an effect"),
    ("exit", "control of the host is an effect"),
    ("random", "a rule must be deterministic"),
    ("time", "a rule must not read the clock"),
    ("now", "a rule must not read the clock"),
];

/// One reason a clause is outside the profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileViolation {
    /// The clause, whitespace collapsed and cut to 120 characters.
    pub clause: String,
    /// Why it is outside the profile.
    pub reason: String,
}

fn strip_comments(text: &str) -> String {
    text.lines()
        .map(|l| {
            let mut in_str = false;
            let mut end = l.len();
            for (i, c) in l.char_indices() {
                match c {
                    '"' => in_str = !in_str,
                    '%' if !in_str => {
                        end = i;
                        break;
                    }
                    _ => {}
                }
            }
            &l[..end]
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The clauses of a program: text up to a full stop that is followed by white space or the end,
/// outside string literals. Declarations (`pred`, `type`, `kind`, `accumulate`, `:`) are dropped.
pub fn clauses(text: &str) -> Vec<String> {
    let s = strip_comments(text);
    let chars: Vec<char> = s.chars().collect();
    let (mut out, mut cur, mut in_str) = (Vec::new(), String::new(), false);
    for (i, &c) in chars.iter().enumerate() {
        cur.push(c);
        if c == '"' {
            in_str = !in_str;
        }
        if c == '.' && !in_str && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|c| {
            !c.is_empty()
                && !["pred ", "type ", "kind ", "accumulate", ":", "macro "]
                    .iter()
                    .any(|p| c.starts_with(p))
        })
        .collect()
}

/// Identifier-like tokens of a clause outside string literals, and the `!` marks.
fn tokens(clause: &str) -> (Vec<String>, bool) {
    let (mut toks, mut cur, mut in_str, mut cut) = (Vec::new(), String::new(), false, false);
    for c in clause.chars() {
        if c == '"' {
            in_str = !in_str;
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if in_str {
            continue;
        }
        if c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '\'') {
            cur.push(c);
        } else {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            if c == '!' {
                cut = true;
            }
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    let toks = toks
        .into_iter()
        .map(|t| t.trim_end_matches('.').to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    (toks, cut)
}

fn short(clause: &str) -> String {
    clause.chars().take(120).collect()
}

/// The head predicate of a clause and its body text.
fn split_clause(clause: &str) -> (String, String) {
    let body_at = clause.find(":-");
    let (head, body) = match body_at {
        Some(i) => (&clause[..i], &clause[i + 2..]),
        None => (clause, ""),
    };
    let name = tokens(head).0.into_iter().next().unwrap_or_default();
    (name, body.to_owned())
}

/// Every reason the program is outside the profile; empty means inside.
// @tg: implements:: [[openspec:rule-verification#Rules Profile]]
pub fn check_profile(text: &str) -> Vec<ProfileViolation> {
    let mut out = Vec::new();
    let cs = clauses(text);
    let mut heads: BTreeSet<String> = BTreeSet::new();
    let mut edges: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for c in &cs {
        let (head, _) = split_clause(c);
        if !head.is_empty() {
            heads.insert(head);
        }
    }
    for c in &cs {
        let (toks, cut) = tokens(c);
        if cut {
            out.push(ProfileViolation {
                clause: short(c),
                reason: "cut makes the meaning depend on the order of clauses".to_owned(),
            });
        }
        for t in &toks {
            if let Some((_, why)) = FORBIDDEN.iter().find(|(w, _)| *w == t) {
                out.push(ProfileViolation {
                    clause: short(c),
                    reason: format!("`{t}`: {why}"),
                });
            } else if t.starts_with("std.") || t.starts_with("io.") {
                out.push(ProfileViolation {
                    clause: short(c),
                    reason: format!("`{t}` is not a rules-profile builtin"),
                });
            } else if t.starts_with("tm.") && !HOST_BUILTINS.contains(&t.as_str()) {
                out.push(ProfileViolation {
                    clause: short(c),
                    reason: format!("`{t}` is not a host builtin the profile allows"),
                });
            }
        }
        // dependency edges between defined predicates, negative inside `not (...)`
        let (head, body) = split_clause(c);
        if head.is_empty() {
            continue;
        }
        let neg = negated_spans(&body);
        for (i, w) in word_spans(&body) {
            if heads.contains(&w.0) {
                let negative = neg.iter().any(|(a, b)| i >= *a && i < *b);
                edges
                    .entry(head.clone())
                    .or_default()
                    .push((w.0.clone(), negative));
            }
        }
    }
    for (h, es) in &edges {
        for (q, negative) in es {
            if *negative && reaches(&edges, q, h) {
                out.push(ProfileViolation {
                    clause: h.clone(),
                    reason: format!(
                        "negation of `{q}` inside the recursion through `{h}` is not stratified"
                    ),
                });
            }
        }
    }
    out.dedup();
    out
}

/// Byte spans of `not (...)` and `not X` in a body.
fn negated_spans(body: &str) -> Vec<(usize, usize)> {
    let b = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = body[i..].find("not ") {
        let start = i + p;
        let before_ok = start == 0 || !(b[start - 1] as char).is_alphanumeric();
        let rest = start + 4;
        if before_ok && body[rest..].starts_with('(') {
            let mut depth = 0;
            let mut end = rest;
            for (k, c) in body[rest..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = rest + k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push((start, end));
            i = end.max(start + 4);
        } else {
            i = start + 4;
        }
    }
    out
}

/// Words of a body with their byte offsets, outside strings.
fn word_spans(body: &str) -> Vec<(usize, (String,))> {
    let (mut out, mut cur, mut start, mut in_str) = (Vec::new(), String::new(), 0, false);
    for (i, c) in body.char_indices() {
        if c == '"' {
            in_str = !in_str;
            cur.clear();
            continue;
        }
        if in_str {
            continue;
        }
        if c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '\'') {
            if cur.is_empty() {
                start = i;
            }
            cur.push(c);
        } else if !cur.is_empty() {
            out.push((start, (std::mem::take(&mut cur),)));
        }
    }
    if !cur.is_empty() {
        out.push((start, (cur,)));
    }
    out
}

fn reaches(edges: &BTreeMap<String, Vec<(String, bool)>>, from: &str, to: &str) -> bool {
    let (mut seen, mut stack) = (BTreeSet::new(), vec![from.to_owned()]);
    while let Some(n) = stack.pop() {
        if n == to {
            return true;
        }
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Some(es) = edges.get(&n) {
            stack.extend(es.iter().map(|(q, _)| q.clone()));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
% a comment
tm.conclude (triple C (iri "urn:vt:decision") (iri "urn:vt:event:approve")) [X, E] :-
  tm.triple (iri "urn:vt:ctx") (iri "urn:vt:deciding") C X,
  tm.triple C (iri "urn:vt:ctx:amount") (num A) E, A =< 50000.5.
"#;

    // @tg: verifies:: [[openspec:rule-verification#Rules Profile#In profile]] {level: unit}
    #[test]
    fn a_plain_program_is_inside_the_profile() {
        assert_eq!(check_profile(GOOD), vec![]);
    }

    // @tg: verifies:: [[openspec:rule-verification#Rules Profile#Out of profile]] {level: unit}
    #[test]
    fn effects_cut_findall_and_unknown_builtins_are_outside() {
        for (src, needle) in [
            ("p X :- q X, !, r X.", "cut"),
            ("p X :- std.findall (q Y) L, r L X.", "findall"),
            ("p X :- print X.", "print"),
            ("p X :- std.map X Y.", "std.map"),
            ("p X :- tm.unknown X.", "tm.unknown"),
            ("p X :- time T, q T X.", "time"),
        ] {
            let v = check_profile(src);
            assert!(v.iter().any(|x| x.reason.contains(needle)), "{src}: {v:?}");
        }
    }

    #[test]
    fn strings_and_comments_do_not_count() {
        let src = "% findall print\nq X :- tm.triple X (iri \"urn:vt:findall!\") _ _.\n";
        assert_eq!(check_profile(src), vec![]);
    }

    #[test]
    fn negation_through_recursion_is_not_stratified() {
        let bad = "win X :- move X Y, not (win Y).\nmove a b.\n";
        let v = check_profile(bad);
        assert!(
            v.iter().any(|x| x.reason.contains("not stratified")),
            "{v:?}"
        );
        let ok = "safe X :- node X, not (bad X).\nbad X :- tm.triple X (iri \"urn:vt:p\") _ _.\nnode a.\n";
        assert_eq!(check_profile(ok), vec![]);
    }
}
