//! The compiled chart: a validated IR flattened into dense tables for the step kernel.
//!
//! States are numbered in document order with the `<scxml>` root at index [`ROOT`]; because the
//! numbering is a pre-order walk, the descendants of a state occupy the contiguous index range
//! `i + 1 ..= last`, so `isDescendant` is two comparisons. Ascending index order is entry order
//! and descending index order is exit order. Every compound state (and the root) gets a
//! synthesized default-entry transition resolved by [`crate::validate::resolve_initial`], and
//! transition domains are precomputed whenever they do not depend on history. The compiled
//! chart is derived data and is never serialized; only the IR is.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::bitset::StateSet;
use crate::ir::{
    Binding, Block, Chart, Data, DoneData, Expr, Invoke, StateElement, StateKind, TransitionType,
};
use crate::kernel;
use crate::validate::{Flat, Violation, resolve_initial, resolve_root_initial, validate};

/// Dense index of a state; [`ROOT`] is the `<scxml>` element.
pub type StateIdx = usize;
/// Dense index of a transition in [`CompiledChart::transitions`].
pub type TransIdx = usize;
/// Index of the `<scxml>` root.
pub const ROOT: StateIdx = 0;

/// A state in the compiled chart.
#[derive(Clone, Debug, PartialEq)]
pub struct CState {
    /// The state identifier; empty for the root.
    pub id: String,
    /// The kind; the root is [`StateKind::Compound`].
    pub kind: StateKind,
    /// The parent; `None` only for the root.
    pub parent: Option<StateIdx>,
    /// The last index of this state's subtree (itself when it has no descendants).
    pub last: StateIdx,
    /// Child states that are not history states, in document order.
    pub children: Vec<StateIdx>,
    /// History children in document order.
    pub history: Vec<StateIdx>,
    /// Transitions declared on the state, in document order (for a history state: its default).
    pub transitions: Vec<TransIdx>,
    /// The default-entry transition of a compound state or the root.
    pub initial: Option<TransIdx>,
    /// `onentry` blocks.
    pub onentry: Vec<Block>,
    /// `onexit` blocks.
    pub onexit: Vec<Block>,
    /// Data items declared by the state (for the root: the top-level datamodel).
    pub data: Vec<Data>,
    /// Invokes in document order.
    pub invokes: Vec<Invoke>,
    /// `donedata` of a final state.
    pub donedata: Option<DoneData>,
}

/// The transition domain, precomputed when it does not depend on history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// The transition has no targets and exits nothing.
    Targetless,
    /// The domain is fixed.
    Static(StateIdx),
    /// A target is a history state, so the domain depends on the recorded history.
    Dynamic,
}

/// A transition in the compiled chart.
#[derive(Clone, Debug, PartialEq)]
pub struct CTransition {
    /// The source state.
    pub source: StateIdx,
    /// Index in the source's IR `transitions`, or `None` for a synthesized default entry.
    pub index: Option<usize>,
    /// Event descriptors; empty for an eventless transition.
    pub events: Vec<String>,
    /// The guard.
    pub cond: Option<Expr>,
    /// Targets in document order of the source.
    pub targets: Vec<StateIdx>,
    /// Whether the transition is of type internal.
    pub internal: bool,
    /// Executable content.
    pub body: Block,
    /// The precomputed domain.
    pub domain: Domain,
}

/// A validated chart flattened into tables for the step kernel.
#[derive(Clone, Debug)]
pub struct CompiledChart {
    chart: Chart,
    states: Vec<CState>,
    transitions: Vec<CTransition>,
    by_id: BTreeMap<String, StateIdx>,
}

impl CompiledChart {
    /// Validate and compile a chart. Fails with every well-formedness violation.
    pub fn new(chart: Chart) -> Result<Self, Vec<Violation>> {
        validate(&chart)?;
        Ok(Self::build(chart))
    }

    fn build(chart: Chart) -> Self {
        let flat = Flat::new(&chart);
        let n = flat.states.len() + 1;
        let mut states: Vec<CState> = Vec::with_capacity(n);
        let mut by_id = BTreeMap::new();
        states.push(CState {
            id: String::new(),
            kind: StateKind::Compound,
            parent: None,
            last: n - 1,
            children: Vec::new(),
            history: Vec::new(),
            transitions: Vec::new(),
            initial: None,
            onentry: Vec::new(),
            onexit: Vec::new(),
            data: chart.data.clone(),
            invokes: Vec::new(),
            donedata: None,
        });
        for (i, s) in flat.states.iter().enumerate() {
            let idx = i + 1;
            let parent = flat.parent[i].map(|p| p + 1).unwrap_or(ROOT);
            by_id.insert(s.id.clone(), idx);
            states.push(CState {
                id: s.id.clone(),
                kind: s.kind(),
                parent: Some(parent),
                last: flat.last[i] + 1,
                children: Vec::new(),
                history: Vec::new(),
                transitions: Vec::new(),
                initial: None,
                onentry: s.onentry.clone(),
                onexit: s.onexit.clone(),
                data: s.data.clone(),
                invokes: s.invokes.clone(),
                donedata: s.donedata.clone(),
            });
            if matches!(s.element, StateElement::History(_)) {
                states[parent].history.push(idx);
            } else {
                states[parent].children.push(idx);
            }
        }
        let resolve = |ids: &[&str]| -> Vec<StateIdx> {
            ids.iter()
                .filter_map(|id| by_id.get(*id).copied())
                .collect()
        };

        let mut transitions: Vec<CTransition> = Vec::new();
        // Root default entry.
        let root_init = resolve_root_initial(&chart);
        transitions.push(CTransition {
            source: ROOT,
            index: None,
            events: Vec::new(),
            cond: None,
            targets: resolve(&root_init.targets),
            internal: true,
            body: Block::new(),
            domain: Domain::Static(ROOT),
        });
        states[ROOT].initial = Some(0);

        for (i, s) in flat.states.iter().enumerate() {
            let idx = i + 1;
            if s.kind() == StateKind::Compound {
                let r = resolve_initial(s);
                let t = transitions.len();
                transitions.push(CTransition {
                    source: idx,
                    index: None,
                    events: Vec::new(),
                    cond: None,
                    targets: resolve(&r.targets),
                    internal: true,
                    body: r.body.cloned().unwrap_or_default(),
                    domain: Domain::Static(idx),
                });
                states[idx].initial = Some(t);
            }
            for (ti, tr) in s.transitions.iter().enumerate() {
                let t = transitions.len();
                transitions.push(CTransition {
                    source: idx,
                    index: Some(ti),
                    events: tr.events.clone(),
                    cond: tr.cond.clone(),
                    targets: tr
                        .targets
                        .iter()
                        .filter_map(|id| by_id.get(id.as_str()).copied())
                        .collect(),
                    internal: tr.kind == TransitionType::Internal,
                    body: tr.body.clone(),
                    domain: Domain::Targetless,
                });
                states[idx].transitions.push(t);
            }
        }

        // A history state without a default transition defaults to its parent's default entry
        // (compound parent) or to all regions (parallel parent).
        for h in 0..states.len() {
            if matches!(states[h].kind, StateKind::History(_)) && states[h].transitions.is_empty() {
                let p = states[h].parent.unwrap_or(ROOT);
                let targets = match states[p].initial {
                    Some(ti) => transitions[ti].targets.clone(),
                    None => states[p].children.clone(),
                };
                let t = transitions.len();
                transitions.push(CTransition {
                    source: h,
                    index: None,
                    events: Vec::new(),
                    cond: None,
                    targets,
                    internal: false,
                    body: Block::new(),
                    domain: Domain::Targetless,
                });
                states[h].transitions.push(t);
            }
        }

        let mut cc = CompiledChart {
            chart,
            states,
            transitions,
            by_id,
        };
        // Precompute domains that do not depend on history.
        let empty = kernel::History::new(cc.states.len());
        for t in 0..cc.transitions.len() {
            let tr = &cc.transitions[t];
            // Default-entry transitions keep their owning state as domain.
            let default_entry =
                tr.index.is_none() && !matches!(cc.states[tr.source].kind, StateKind::History(_));
            if default_entry {
                continue;
            }
            let domain = if tr.targets.is_empty() {
                Domain::Targetless
            } else if tr
                .targets
                .iter()
                .any(|s| matches!(cc.states[*s].kind, StateKind::History(_)))
            {
                Domain::Dynamic
            } else {
                match kernel::compute_domain(&cc, &empty, t) {
                    Some(d) => Domain::Static(d),
                    None => Domain::Targetless,
                }
            };
            cc.transitions[t].domain = domain;
        }
        cc
    }

    /// The IR this chart was compiled from.
    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    /// The datamodel name the chart requires (`"null"` when it names none).
    pub fn datamodel(&self) -> &str {
        if self.chart.datamodel.is_empty() {
            "null"
        } else {
            &self.chart.datamodel
        }
    }

    /// The binding mode.
    pub fn binding(&self) -> Binding {
        self.chart.binding
    }

    /// All states; index [`ROOT`] is the root.
    pub fn states(&self) -> &[CState] {
        &self.states
    }

    /// One state.
    pub fn state(&self, i: StateIdx) -> &CState {
        &self.states[i]
    }

    /// All transitions.
    pub fn transitions(&self) -> &[CTransition] {
        &self.transitions
    }

    /// One transition.
    pub fn transition(&self, t: TransIdx) -> &CTransition {
        &self.transitions[t]
    }

    /// The number of states including the root.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Whether the chart has no states besides the root (never true for a valid chart).
    pub fn is_empty(&self) -> bool {
        self.states.len() <= 1
    }

    /// The index of the state with this identifier.
    pub fn index_of(&self, id: &str) -> Option<StateIdx> {
        self.by_id.get(id).copied()
    }

    /// Whether `d` is a proper descendant of `a`.
    pub fn is_descendant(&self, d: StateIdx, a: StateIdx) -> bool {
        a < d && d <= self.states[a].last
    }

    /// The resolved default entry of a compound state or the root, as state indices.
    pub fn default_entry(&self, s: StateIdx) -> &[StateIdx] {
        match self.states[s].initial {
            Some(t) => &self.transitions[t].targets,
            None => &[],
        }
    }

    /// An empty configuration sized for this chart.
    pub fn empty_set(&self) -> StateSet {
        StateSet::new(self.states.len())
    }

    /// State identifiers of a set, in document order.
    pub fn ids(&self, set: &StateSet) -> Vec<String> {
        set.to_vec()
            .into_iter()
            .map(|i| self.states[i].id.clone())
            .collect()
    }
}
