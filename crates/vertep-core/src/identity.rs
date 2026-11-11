//! Deterministic identifiers for states declared without an `id`.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ir::{Chart, State};

/// Prefix of every generated identifier. Declared identifiers may still use it; collisions are
/// resolved by suffixing.
pub const GENERATED_PREFIX: &str = "_vt.s";

/// Give every state with an empty `id` a generated identifier and mark it `id_generated`.
///
/// The identifier is `_vt.s.<path>` where `<path>` is the dot-joined list of 0-based child
/// positions from the root, so it depends only on document position. When it collides with a
/// declared identifier, `_` is appended until it does not.
// @tg: implements:: [[openspec:chart-model#State Identity]]
pub fn assign_generated_ids(chart: &mut Chart) {
    let mut taken = BTreeSet::new();
    collect_declared(&chart.states, &mut taken);
    let mut path = Vec::new();
    for (i, s) in chart.states.iter_mut().enumerate() {
        path.push(i);
        assign(s, &mut path, &mut taken);
        path.pop();
    }
}

fn collect_declared(states: &[State], taken: &mut BTreeSet<String>) {
    for s in states {
        if !s.id.is_empty() && !s.id_generated {
            taken.insert(s.id.clone());
        }
        collect_declared(&s.children, taken);
    }
}

fn assign(state: &mut State, path: &mut Vec<usize>, taken: &mut BTreeSet<String>) {
    if state.id.is_empty() {
        let mut id = String::from(GENERATED_PREFIX);
        for p in path.iter() {
            id.push_str(&format!(".{p}"));
        }
        while taken.contains(&id) {
            id.push('_');
        }
        taken.insert(id.clone());
        state.id = id;
        state.id_generated = true;
    }
    for (i, c) in state.children.iter_mut().enumerate() {
        path.push(i);
        assign(c, path, taken);
        path.pop();
    }
}
