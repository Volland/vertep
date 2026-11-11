//! State identifiers: recognizing written-out generated identifiers, collecting references and
//! checking that references resolve.

use std::collections::{BTreeMap, BTreeSet};

use vertep_core::identity::GENERATED_PREFIX;
use vertep_core::ir::{Chart, Span, State};

use crate::error::{ErrorKind, ParseError};

fn base_id(path: &[usize]) -> String {
    let mut id = String::from(GENERATED_PREFIX);
    for p in path {
        id.push('.');
        id.push_str(&p.to_string());
    }
    id
}

/// Whether `id` has the generated form for the state at `path`: the base identifier followed
/// by zero or more `_`.
fn has_own_generated_form(id: &str, path: &[usize]) -> bool {
    let base = base_id(path);
    id.strip_prefix(base.as_str())
        .is_some_and(|rest| rest.chars().all(|c| c == '_'))
}

/// Mark declared identifiers that are exactly what [`vertep_core::identity::assign_generated_ids`]
/// would generate for their position as generated.
///
/// The writer writes a generated identifier when a transition targets the state; reading that
/// document back must yield a generated identifier again, or the round trip would turn it into
/// a declared one. The test is positional: an identifier `_vt.s.<path>` (plus the collision
/// suffix the generator would choose) on the state at `<path>`.
pub(crate) fn recognize_generated(chart: &mut Chart) {
    fn collect(
        states: &[State],
        path: &mut Vec<usize>,
        taken: &mut BTreeSet<String>,
        candidates: &mut Vec<Vec<usize>>,
    ) {
        for (i, s) in states.iter().enumerate() {
            path.push(i);
            if !s.id_generated && !s.id.is_empty() {
                if has_own_generated_form(&s.id, path) {
                    candidates.push(path.clone());
                } else {
                    taken.insert(s.id.clone());
                }
            }
            collect(&s.children, path, taken, candidates);
            path.pop();
        }
    }
    let mut taken = BTreeSet::new();
    let mut candidates = Vec::new();
    collect(&chart.states, &mut Vec::new(), &mut taken, &mut candidates);
    for path in candidates {
        let mut expected = base_id(&path);
        while taken.contains(&expected) {
            expected.push('_');
        }
        if let Some(s) = state_at_mut(&mut chart.states, &path)
            && s.id == expected
        {
            s.id_generated = true;
        }
    }
}

fn state_at_mut<'c>(states: &'c mut [State], path: &[usize]) -> Option<&'c mut State> {
    let (first, rest) = path.split_first()?;
    let s = states.get_mut(*first)?;
    if rest.is_empty() {
        Some(s)
    } else {
        state_at_mut(&mut s.children, rest)
    }
}

/// Every state identifier the chart refers to: transition targets (including `<initial>` and
/// history defaults) and `initial` attributes.
pub(crate) fn referenced_ids(chart: &Chart) -> BTreeSet<String> {
    fn walk(states: &[State], out: &mut BTreeSet<String>) {
        for s in states {
            out.extend(s.initial.iter().cloned());
            if let Some(i) = &s.initial_element {
                out.extend(i.transition.targets.iter().cloned());
            }
            for t in &s.transitions {
                out.extend(t.targets.iter().cloned());
            }
            walk(&s.children, out);
        }
    }
    let mut out: BTreeSet<String> = chart.initial.iter().cloned().collect();
    walk(&chart.states, &mut out);
    out
}

/// Duplicate declared identifiers and references to identifiers no state has.
///
/// This is the part of well-formedness the reader checks itself so that such errors carry the
/// source location of the offending element; the remaining structural rules are checked by
/// the core validator.
pub(crate) fn check_references(chart: &Chart) -> Vec<ParseError> {
    fn collect(states: &[State], seen: &mut BTreeMap<String, Span>, errors: &mut Vec<ParseError>) {
        for s in states {
            if let Some(first) = seen.get(&s.id) {
                errors.push(ParseError {
                    kind: ErrorKind::DuplicateStateId,
                    span: s.span,
                    message: format!(
                        "state id `{}` is already declared at {}:{}",
                        s.id, first.line, first.column
                    ),
                });
            } else {
                seen.insert(s.id.clone(), s.span);
            }
            collect(&s.children, seen, errors);
        }
    }
    fn unknown(
        ids: &BTreeMap<String, Span>,
        refs: &[String],
        span: Span,
        what: &str,
    ) -> Vec<ParseError> {
        refs.iter()
            .filter(|r| !ids.contains_key(*r))
            .map(|r| ParseError {
                kind: ErrorKind::UnknownTarget,
                span,
                message: format!("{what} names unknown state `{r}`"),
            })
            .collect()
    }
    fn refs(states: &[State], ids: &BTreeMap<String, Span>, errors: &mut Vec<ParseError>) {
        for s in states {
            errors.extend(unknown(ids, &s.initial, s.span, "`initial` attribute"));
            if let Some(i) = &s.initial_element {
                errors.extend(unknown(
                    ids,
                    &i.transition.targets,
                    i.transition.span,
                    "<initial> transition",
                ));
            }
            for t in &s.transitions {
                errors.extend(unknown(ids, &t.targets, t.span, "transition target"));
            }
            refs(&s.children, ids, errors);
        }
    }
    let mut errors = Vec::new();
    let mut ids = BTreeMap::new();
    collect(&chart.states, &mut ids, &mut errors);
    errors.extend(unknown(
        &ids,
        &chart.initial,
        chart.span,
        "`initial` attribute",
    ));
    refs(&chart.states, &ids, &mut errors);
    errors
}
