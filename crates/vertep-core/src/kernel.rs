//! The selection and entry/exit kernel of the W3C SCXML interpretation algorithm (Appendix D).
//!
//! Every function here is a pure function over a [`CompiledChart`], a configuration
//! ([`StateSet`]) and the recorded [`History`]. The names follow Appendix D:
//! [`select_transitions`] (`selectTransitions` and `selectEventlessTransitions`),
//! [`remove_conflicting_transitions`], [`compute_exit_set`], [`compute_entry_set`]
//! (`computeEntrySet`, `addDescendantStatesToEnter`, `addAncestorStatesToEnter`),
//! [`transition_domain`] (`getTransitionDomain`), [`find_lcca`], [`effective_targets`]
//! (`getEffectiveTargetStates`), [`record_history`] (the history step of `exitStates`) and
//! [`is_in_final_state`].
//!
//! # Aeneas-friendly boundary
//!
//! The kernel stays inside the Rust subset that Aeneas translates
//! ([[constraints#Aeneas supports a Rust subset]]): it uses no trait objects, no interior
//! mutability (`Cell`, `RefCell`), no `unsafe`, no async and no I/O. Data is plain vectors,
//! bitsets of words and `usize` indices; iteration is over owned index vectors. The only
//! abstraction is the generic condition oracle `FnMut(TransIdx) -> bool` taken by
//! [`select_transitions`], which is monomorphized, never dynamically dispatched.
//!
//! Executable content and condition evaluation are outside the kernel: the session driver in
//! [`crate::session`] calls the [`crate::datamodel::Datamodel`] trait object and applies the
//! kernel's plans (exit order, entry order, default entries, default history content) one
//! state at a time, so the order of datamodel requests is fully determined by the kernel's
//! output. That order is what the datamodel tape ([`crate::tape`]) records for the Lean oracle.

use alloc::vec::Vec;

use crate::bitset::StateSet;
use crate::compile::{CompiledChart, Domain, ROOT, StateIdx, TransIdx};
use crate::ir::StateKind;

/// The history value table: for each history state, the states recorded when its parent was
/// last exited, or `None` when it was never exited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct History {
    values: Vec<Option<StateSet>>,
}

impl History {
    /// An empty table for a chart of `n` states.
    pub fn new(n: usize) -> Self {
        History {
            values: alloc::vec![None; n],
        }
    }

    /// The recorded value of history state `h`.
    pub fn get(&self, h: StateIdx) -> Option<&StateSet> {
        self.values.get(h).and_then(|v| v.as_ref())
    }

    /// Record a value for history state `h`.
    pub fn set(&mut self, h: StateIdx, value: StateSet) {
        if h >= self.values.len() {
            self.values.resize(h + 1, None);
        }
        self.values[h] = Some(value);
    }
}

/// Whether one event descriptor matches an event name (W3C SCXML 3.12.1): `*` matches every
/// event; otherwise, after dropping a trailing `.*` or `.`, the descriptor must equal the name or
/// be a prefix of it that ends at a `.` token boundary.
// @tg: implements:: [[openspec:scxml-interpretation#Event Descriptor Matching]]
pub fn descriptor_matches(descriptor: &str, name: &str) -> bool {
    let d = descriptor.trim();
    if d == "*" {
        return true;
    }
    let d = d.strip_suffix(".*").unwrap_or(d);
    let d = d.strip_suffix('.').unwrap_or(d);
    if d.is_empty() {
        return false;
    }
    if name == d {
        return true;
    }
    name.len() > d.len() && name.starts_with(d) && name.as_bytes()[d.len()] == b'.'
}

/// Whether any of a transition's descriptors matches the event name.
pub fn name_match(descriptors: &[alloc::string::String], name: &str) -> bool {
    descriptors.iter().any(|d| descriptor_matches(d, name))
}

/// Whether a state is atomic: it has no child states (history children do not count).
pub fn is_atomic(chart: &CompiledChart, s: StateIdx) -> bool {
    s != ROOT
        && chart.state(s).children.is_empty()
        && !matches!(chart.state(s).kind, StateKind::History(_))
}

/// The proper ancestors of `s` from its parent upward, stopping before `stop`; with no `stop`,
/// up to and including the root.
pub fn proper_ancestors(
    chart: &CompiledChart,
    s: StateIdx,
    stop: Option<StateIdx>,
) -> Vec<StateIdx> {
    let mut out = Vec::new();
    let mut cur = chart.state(s).parent;
    while let Some(p) = cur {
        if Some(p) == stop {
            break;
        }
        out.push(p);
        cur = chart.state(p).parent;
    }
    out
}

fn push_unique(v: &mut Vec<StateIdx>, s: StateIdx) {
    if !v.contains(&s) {
        v.push(s);
    }
}

/// `getEffectiveTargetStates`: the targets of `t` with history states replaced by their recorded
/// states, or by the effective targets of their default transition when nothing is recorded.
pub fn effective_targets(chart: &CompiledChart, history: &History, t: TransIdx) -> Vec<StateIdx> {
    let mut out = Vec::new();
    for &s in &chart.transition(t).targets {
        if matches!(chart.state(s).kind, StateKind::History(_)) {
            match history.get(s) {
                Some(v) => {
                    for x in v.to_vec() {
                        push_unique(&mut out, x);
                    }
                }
                None => {
                    if let Some(&dt) = chart.state(s).transitions.first() {
                        for x in effective_targets(chart, history, dt) {
                            push_unique(&mut out, x);
                        }
                    }
                }
            }
        } else {
            push_unique(&mut out, s);
        }
    }
    out
}

/// `findLCCA`: the nearest proper ancestor of `head` that is compound (or the root) and has every
/// state of `tail` as a descendant.
pub fn find_lcca(chart: &CompiledChart, head: StateIdx, tail: &[StateIdx]) -> StateIdx {
    for anc in proper_ancestors(chart, head, None) {
        if chart.state(anc).kind == StateKind::Compound
            && tail.iter().all(|s| chart.is_descendant(*s, anc))
        {
            return anc;
        }
    }
    ROOT
}

/// `getTransitionDomain`, computed from scratch.
pub fn compute_domain(chart: &CompiledChart, history: &History, t: TransIdx) -> Option<StateIdx> {
    let tstates = effective_targets(chart, history, t);
    if tstates.is_empty() {
        return None;
    }
    let tr = chart.transition(t);
    if tr.internal
        && chart.state(tr.source).kind == StateKind::Compound
        && tstates.iter().all(|s| chart.is_descendant(*s, tr.source))
    {
        return Some(tr.source);
    }
    Some(find_lcca(chart, tr.source, &tstates))
}

/// `getTransitionDomain`, using the precomputed domain when it does not depend on history.
// @tg: implements:: [[openspec:scxml-interpretation#Internal Transitions]]
pub fn transition_domain(
    chart: &CompiledChart,
    history: &History,
    t: TransIdx,
) -> Option<StateIdx> {
    match chart.transition(t).domain {
        Domain::Targetless => None,
        Domain::Static(d) => Some(d),
        Domain::Dynamic => compute_domain(chart, history, t),
    }
}

/// `computeExitSet`: the active states that are proper descendants of the domains of the
/// targeted transitions in `ts`.
pub fn compute_exit_set(
    chart: &CompiledChart,
    config: &StateSet,
    history: &History,
    ts: &[TransIdx],
) -> StateSet {
    let mut out = chart.empty_set();
    for &t in ts {
        if chart.transition(t).targets.is_empty() {
            continue;
        }
        if let Some(d) = transition_domain(chart, history, t) {
            for s in config.to_vec() {
                if chart.is_descendant(s, d) {
                    out.insert(s);
                }
            }
        }
    }
    out
}

/// `removeConflictingTransitions`: keep a transition unless its exit set intersects that of a
/// transition kept earlier; a transition whose source descends from the earlier one's source
/// replaces it, otherwise the earlier one wins.
pub fn remove_conflicting_transitions(
    chart: &CompiledChart,
    config: &StateSet,
    history: &History,
    enabled: &[TransIdx],
) -> Vec<TransIdx> {
    let mut filtered: Vec<TransIdx> = Vec::new();
    for &t1 in enabled {
        let ex1 = compute_exit_set(chart, config, history, &[t1]);
        let mut preempted = false;
        let mut to_remove: Vec<TransIdx> = Vec::new();
        for &t2 in &filtered {
            let ex2 = compute_exit_set(chart, config, history, &[t2]);
            if ex1.intersects(&ex2) {
                if chart.is_descendant(chart.transition(t1).source, chart.transition(t2).source) {
                    to_remove.push(t2);
                } else {
                    preempted = true;
                    break;
                }
            }
        }
        if !preempted {
            filtered.retain(|t| !to_remove.contains(t));
            filtered.push(t1);
        }
    }
    filtered
}

/// `selectTransitions` (with `Some(event)`) and `selectEventlessTransitions` (with `None`): visit
/// the active atomic states in document order and, for each, take the first transition in
/// document order on the state or its nearest ancestor that matches the event and whose
/// condition holds; then remove conflicts. `cond` is called, in this order, for each candidate
/// transition that has a condition, and answers whether it holds.
// @tg: implements:: [[openspec:scxml-interpretation#Optimal Enabled Transition Set]]
pub fn select_transitions<F: FnMut(TransIdx) -> bool>(
    chart: &CompiledChart,
    config: &StateSet,
    history: &History,
    event: Option<&str>,
    mut cond: F,
) -> Vec<TransIdx> {
    let mut enabled: Vec<TransIdx> = Vec::new();
    for state in config.to_vec() {
        if !is_atomic(chart, state) {
            continue;
        }
        let mut chain = alloc::vec![state];
        chain.extend(proper_ancestors(chart, state, None));
        'outer: for s in chain {
            for &t in &chart.state(s).transitions {
                let tr = chart.transition(t);
                let matches = match event {
                    None => tr.events.is_empty(),
                    Some(name) => !tr.events.is_empty() && name_match(&tr.events, name),
                };
                if matches && (tr.cond.is_none() || cond(t)) {
                    if !enabled.contains(&t) {
                        enabled.push(t);
                    }
                    break 'outer;
                }
            }
        }
    }
    remove_conflicting_transitions(chart, config, history, &enabled)
}

/// The history step of `exitStates`: for every state in the exit set with history children,
/// record the active atomic descendants (deep) or active children (shallow).
pub fn record_history(
    chart: &CompiledChart,
    config: &StateSet,
    exit_set: &StateSet,
    history: &mut History,
) {
    for s in exit_set.to_vec() {
        for &h in &chart.state(s).history {
            let deep = matches!(
                chart.state(h).kind,
                StateKind::History(crate::ir::HistoryType::Deep)
            );
            let mut v = chart.empty_set();
            for s0 in config.to_vec() {
                let keep = if deep {
                    is_atomic(chart, s0) && chart.is_descendant(s0, s)
                } else {
                    chart.state(s0).parent == Some(s)
                };
                if keep {
                    v.insert(s0);
                }
            }
            history.set(h, v);
        }
    }
}

/// The result of `computeEntrySet`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryPlan {
    /// States to enter; enter them in ascending (document) order.
    pub states: StateSet,
    /// Compound states entered by default, whose default-entry content runs after `onentry`.
    pub default_entry: StateSet,
    /// For history parents entered through a history's default transition: the parent and that
    /// transition, whose content runs after the parent's `onentry`.
    pub default_history_content: Vec<(StateIdx, TransIdx)>,
}

/// `computeEntrySet`.
// @tg: implements:: [[openspec:scxml-interpretation#History Restoration]]
pub fn compute_entry_set(chart: &CompiledChart, history: &History, ts: &[TransIdx]) -> EntryPlan {
    let mut p = EntryPlan {
        states: chart.empty_set(),
        default_entry: chart.empty_set(),
        default_history_content: Vec::new(),
    };
    for &t in ts {
        for &s in &chart.transition(t).targets {
            add_descendants(chart, history, s, &mut p);
        }
        let ancestor = transition_domain(chart, history, t);
        for s in effective_targets(chart, history, t) {
            add_ancestors(chart, history, s, ancestor, &mut p);
        }
    }
    p
}

fn add_descendants(chart: &CompiledChart, history: &History, state: StateIdx, p: &mut EntryPlan) {
    let st = chart.state(state);
    if matches!(st.kind, StateKind::History(_)) {
        let parent = st.parent.unwrap_or(ROOT);
        match history.get(state) {
            Some(v) => {
                let v = v.to_vec();
                for &s in &v {
                    add_descendants(chart, history, s, p);
                }
                for &s in &v {
                    add_ancestors(chart, history, s, Some(parent), p);
                }
            }
            None => {
                if let Some(&dt) = st.transitions.first() {
                    p.default_history_content.retain(|(par, _)| *par != parent);
                    p.default_history_content.push((parent, dt));
                    for &s in &chart.transition(dt).targets {
                        add_descendants(chart, history, s, p);
                    }
                    for &s in &chart.transition(dt).targets {
                        add_ancestors(chart, history, s, Some(parent), p);
                    }
                }
            }
        }
        return;
    }
    p.states.insert(state);
    match st.kind {
        StateKind::Compound => {
            p.default_entry.insert(state);
            if let Some(it) = st.initial {
                for &s in &chart.transition(it).targets {
                    add_descendants(chart, history, s, p);
                }
                for &s in &chart.transition(it).targets {
                    add_ancestors(chart, history, s, Some(state), p);
                }
            }
        }
        StateKind::Parallel => {
            for &child in &st.children {
                if !p.states.any_in_range(child + 1, chart.state(child).last) {
                    add_descendants(chart, history, child, p);
                }
            }
        }
        _ => {}
    }
}

fn add_ancestors(
    chart: &CompiledChart,
    history: &History,
    state: StateIdx,
    ancestor: Option<StateIdx>,
    p: &mut EntryPlan,
) {
    for anc in proper_ancestors(chart, state, ancestor) {
        if anc == ROOT {
            continue;
        }
        p.states.insert(anc);
        if chart.state(anc).kind == StateKind::Parallel {
            for &child in &chart.state(anc).children {
                if !p.states.any_in_range(child + 1, chart.state(child).last) {
                    add_descendants(chart, history, child, p);
                }
            }
        }
    }
}

/// `isInFinalState`: a compound state with an active final child, or a parallel state all of
/// whose regions are in a final state.
pub fn is_in_final_state(chart: &CompiledChart, config: &StateSet, s: StateIdx) -> bool {
    let st = chart.state(s);
    match st.kind {
        StateKind::Compound => st
            .children
            .iter()
            .any(|c| chart.state(*c).kind == StateKind::Final && config.contains(*c)),
        StateKind::Parallel => st
            .children
            .iter()
            .all(|c| is_in_final_state(chart, config, *c)),
        _ => false,
    }
}
