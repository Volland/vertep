//! Well-formedness validation and initial-state resolution.
//!
//! [`validate`] checks every rule and returns all violations together, each naming the offending
//! node; a chart that passes can be compiled with [`crate::compile::CompiledChart::new`] and run.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ir::{Block, Chart, Exec, ExecKind, Param, Span, State, StateElement, StateKind};

/// Where a resolved default entry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialSource {
    /// The `initial` attribute.
    Attribute,
    /// The transition of the `<initial>` element.
    Element,
    /// The first child state in document order.
    FirstChild,
    /// The state has no default entry (it is not compound, or has no child states).
    None,
}

/// The resolved default entry of a compound state or of the root.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedInitial<'a> {
    /// The identifiers of the default entry targets, in document order of the source.
    pub targets: Vec<&'a str>,
    /// Where the targets came from.
    pub source: InitialSource,
    /// The executable content of the `<initial>` transition, if it came from an element.
    pub body: Option<&'a Block>,
}

/// Resolve the default entry of a state: the `initial` attribute, else the `<initial>` element's
/// transition, else the first child state (not a history pseudo-state) in document order.
// @tg: implements:: [[openspec:chart-model#Initial State Resolution]]
pub fn resolve_initial(state: &State) -> ResolvedInitial<'_> {
    if state.kind() != StateKind::Compound {
        return ResolvedInitial {
            targets: Vec::new(),
            source: InitialSource::None,
            body: None,
        };
    }
    if !state.initial.is_empty() {
        return ResolvedInitial {
            targets: state.initial.iter().map(String::as_str).collect(),
            source: InitialSource::Attribute,
            body: None,
        };
    }
    if let Some(init) = &state.initial_element {
        return ResolvedInitial {
            targets: init.transition.targets.iter().map(String::as_str).collect(),
            source: InitialSource::Element,
            body: Some(&init.transition.body),
        };
    }
    first_child(&state.children)
}

/// Resolve the default entry of the root: its `initial` attribute, else its first child state.
pub fn resolve_root_initial(chart: &Chart) -> ResolvedInitial<'_> {
    if !chart.initial.is_empty() {
        return ResolvedInitial {
            targets: chart.initial.iter().map(String::as_str).collect(),
            source: InitialSource::Attribute,
            body: None,
        };
    }
    first_child(&chart.states)
}

fn first_child(children: &[State]) -> ResolvedInitial<'_> {
    match children
        .iter()
        .find(|c| !matches!(c.element, StateElement::History(_)))
    {
        Some(c) => ResolvedInitial {
            targets: alloc::vec![c.id.as_str()],
            source: InitialSource::FirstChild,
            body: None,
        },
        None => ResolvedInitial {
            targets: Vec::new(),
            source: InitialSource::None,
            body: None,
        },
    }
}

/// The node a violation is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeRef {
    /// The `<scxml>` root.
    Root,
    /// A state, by identifier (possibly empty when the identifier is missing).
    State {
        /// The state identifier.
        id: String,
    },
    /// A transition, by source state and index in its transition list.
    Transition {
        /// The source state identifier.
        source: String,
        /// Index in `State::transitions`.
        index: usize,
    },
    /// The `initial` attribute or `<initial>` element of a state.
    Initial {
        /// The owning state identifier.
        state: String,
    },
    /// A data item, by identifier.
    Data {
        /// The data identifier.
        id: String,
    },
    /// An invoke, by owning state and index.
    Invoke {
        /// The owning state identifier.
        state: String,
        /// Index in `State::invokes`.
        index: usize,
    },
    /// An executable content element inside a state (its handlers, transitions or invokes).
    Exec {
        /// The owning state identifier.
        state: String,
    },
}

/// What is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViolationKind {
    /// The chart has no child states.
    EmptyChart,
    /// A state has no identifier (run [`crate::identity::assign_generated_ids`] first).
    MissingId,
    /// Two states share an identifier.
    DuplicateStateId {
        /// The repeated identifier.
        id: String,
    },
    /// A target names no state.
    UnknownTarget {
        /// The unknown identifier.
        target: String,
    },
    /// An `initial` attribute or `<initial>` transition targets a state that is not a proper
    /// descendant of the owning state.
    IllegalInitialTarget {
        /// The offending target.
        target: String,
    },
    /// A state has both an `initial` attribute and an `<initial>` element.
    InitialAttributeAndElement,
    /// An `initial` attribute or `<initial>` element on a state that is not compound.
    InitialOnNonCompound,
    /// An `<initial>` transition with an event or a condition, or without a target.
    IllegalInitialTransition,
    /// A history state with more than one transition.
    HistoryTransitionCount,
    /// A history default transition with an event or a condition, or without a target.
    IllegalHistoryTransition,
    /// A history default transition target outside the history's parent.
    IllegalHistoryTarget {
        /// The offending target.
        target: String,
    },
    /// A history state that is a child of the root.
    HistoryAtRoot,
    /// A history state with content other than its default transition.
    HistoryContent,
    /// A final state with content other than `onentry`, `onexit` and `donedata`.
    FinalContent {
        /// What it contains.
        what: String,
    },
    /// `donedata` on a state that is not final.
    DoneDataOutsideFinal,
    /// A parallel state whose children are all history states.
    ParallelHistoryOnly,
    /// A `<state>` whose children are all history states.
    StateHistoryOnly,
    /// The targets of one transition cannot be active together.
    IllegalTargetSet {
        /// The first target of the conflicting pair.
        first: String,
        /// The second target of the conflicting pair.
        second: String,
    },
    /// Default history entry never reaches a real state: following the default transitions of
    /// history states (a history state without one defaults to its parent's `initial`) leads
    /// back to this history state, for example `<state initial="h">` whose history child `h`
    /// has no default transition.
    HistoryDefaultCycle {
        /// The history state on the cycle.
        history: String,
    },
    /// Two data items share an identifier.
    DuplicateDataId {
        /// The repeated identifier.
        id: String,
    },
    /// An element has attributes or children that exclude each other, or lacks a required one.
    ConflictingAttributes {
        /// The element name.
        element: String,
        /// The attributes involved.
        attributes: String,
    },
}

/// One well-formedness violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// What is wrong.
    pub kind: ViolationKind,
    /// The offending node.
    pub node: NodeRef,
    /// Source location of the offending node, if known.
    pub span: Span,
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?} at {:?}", self.kind, self.node)
    }
}

/// Flat pre-order view of the state tree used by validation and compilation.
pub(crate) struct Flat<'a> {
    /// States in document order.
    pub states: Vec<&'a State>,
    /// Parent index, `None` for children of the root.
    pub parent: Vec<Option<usize>>,
    /// Index of the last descendant of each state (itself when it has none).
    pub last: Vec<usize>,
    /// Identifier to first index.
    pub by_id: BTreeMap<&'a str, usize>,
}

impl<'a> Flat<'a> {
    pub(crate) fn new(chart: &'a Chart) -> Self {
        let mut f = Flat {
            states: Vec::new(),
            parent: Vec::new(),
            last: Vec::new(),
            by_id: BTreeMap::new(),
        };
        for s in &chart.states {
            f.push(s, None);
        }
        f
    }

    fn push(&mut self, s: &'a State, parent: Option<usize>) -> usize {
        let i = self.states.len();
        self.states.push(s);
        self.parent.push(parent);
        self.last.push(i);
        self.by_id.entry(s.id.as_str()).or_insert(i);
        for c in &s.children {
            let l = self.push(c, Some(i));
            self.last[i] = l;
        }
        self.last[i]
    }

    /// Whether `d` is a proper descendant of `a`.
    pub(crate) fn is_descendant(&self, d: usize, a: usize) -> bool {
        a < d && d <= self.last[a]
    }
}

/// Validate a chart, returning every violation found.
// @tg: implements:: [[openspec:chart-model#Well-Formedness Validation]]
pub fn validate(chart: &Chart) -> Result<(), Vec<Violation>> {
    let flat = Flat::new(chart);
    let mut v = Vec::new();
    let mut push = |kind: ViolationKind, node: NodeRef, span: Span| {
        v.push(Violation { kind, node, span });
    };

    if chart.states.is_empty() {
        push(ViolationKind::EmptyChart, NodeRef::Root, chart.span);
    }

    // Identifiers.
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &flat.states {
        if s.id.is_empty() {
            push(
                ViolationKind::MissingId,
                NodeRef::State { id: String::new() },
                s.span,
            );
            continue;
        }
        let n = seen.entry(s.id.as_str()).or_insert(0);
        *n += 1;
        if *n == 2 {
            push(
                ViolationKind::DuplicateStateId { id: s.id.clone() },
                NodeRef::State { id: s.id.clone() },
                s.span,
            );
        }
    }

    // Root initial.
    for t in &chart.initial {
        if !flat.by_id.contains_key(t.as_str()) {
            push(
                ViolationKind::UnknownTarget { target: t.clone() },
                NodeRef::Root,
                chart.span,
            );
        }
    }
    check_target_set(&flat, &chart.initial, NodeRef::Root, chart.span, &mut push);
    for s in &chart.states {
        if matches!(s.element, StateElement::History(_)) {
            push(
                ViolationKind::HistoryAtRoot,
                NodeRef::State { id: s.id.clone() },
                s.span,
            );
        }
    }

    check_history_defaults(&flat, &mut push);

    // Data identifiers.
    let mut data_seen: BTreeMap<&str, usize> = BTreeMap::new();
    let all_data = chart
        .data
        .iter()
        .chain(flat.states.iter().flat_map(|s| s.data.iter()));
    for d in all_data {
        let n = data_seen.entry(d.id.as_str()).or_insert(0);
        *n += 1;
        if *n == 2 {
            push(
                ViolationKind::DuplicateDataId { id: d.id.clone() },
                NodeRef::Data { id: d.id.clone() },
                d.span,
            );
        }
        // A resolved `src` keeps both the reference and the content it resolved to (the
        // reader's convention), so `src` with `content` is one source, not two.
        let given = [d.src.is_some() || d.content.is_some(), d.expr.is_some()]
            .iter()
            .filter(|b| **b)
            .count();
        if given > 1 {
            push(
                conflict("data", "src, expr, content"),
                NodeRef::Data { id: d.id.clone() },
                d.span,
            );
        }
    }

    for (i, s) in flat.states.iter().enumerate() {
        let sid = || NodeRef::State { id: s.id.clone() };
        let kind = s.kind();

        // Initial attribute and element.
        let has_initial = !s.initial.is_empty() || s.initial_element.is_some();
        if has_initial && kind != StateKind::Compound {
            push(ViolationKind::InitialOnNonCompound, sid(), s.span);
        }
        if !s.initial.is_empty() && s.initial_element.is_some() {
            push(
                ViolationKind::InitialAttributeAndElement,
                NodeRef::Initial {
                    state: s.id.clone(),
                },
                s.span,
            );
        }
        let init_node = || NodeRef::Initial {
            state: s.id.clone(),
        };
        for t in &s.initial {
            match flat.by_id.get(t.as_str()) {
                None => push(
                    ViolationKind::UnknownTarget { target: t.clone() },
                    init_node(),
                    s.span,
                ),
                Some(&ti) if !flat.is_descendant(ti, i) => push(
                    ViolationKind::IllegalInitialTarget { target: t.clone() },
                    init_node(),
                    s.span,
                ),
                _ => {}
            }
        }
        check_target_set(&flat, &s.initial, init_node(), s.span, &mut push);
        if let Some(init) = &s.initial_element {
            let tr = &init.transition;
            if !tr.events.is_empty() || tr.cond.is_some() || tr.targets.is_empty() {
                push(
                    ViolationKind::IllegalInitialTransition,
                    init_node(),
                    init.span,
                );
            }
            for t in &tr.targets {
                match flat.by_id.get(t.as_str()) {
                    None => push(
                        ViolationKind::UnknownTarget { target: t.clone() },
                        init_node(),
                        init.span,
                    ),
                    Some(&ti) if !flat.is_descendant(ti, i) => push(
                        ViolationKind::IllegalInitialTarget { target: t.clone() },
                        init_node(),
                        init.span,
                    ),
                    _ => {}
                }
            }
            check_target_set(&flat, &tr.targets, init_node(), init.span, &mut push);
            check_block(&tr.body, &s.id, &mut push);
        }

        // Transitions.
        for (ti, t) in s.transitions.iter().enumerate() {
            let tnode = || NodeRef::Transition {
                source: s.id.clone(),
                index: ti,
            };
            for target in &t.targets {
                if !flat.by_id.contains_key(target.as_str()) {
                    push(
                        ViolationKind::UnknownTarget {
                            target: target.clone(),
                        },
                        tnode(),
                        t.span,
                    );
                }
            }
            check_target_set(&flat, &t.targets, tnode(), t.span, &mut push);
            check_block(&t.body, &s.id, &mut push);
        }

        // Kind-specific rules.
        match kind {
            StateKind::History(_) => {
                if s.transitions.len() > 1 {
                    push(ViolationKind::HistoryTransitionCount, sid(), s.span);
                }
                if let Some(t) = s.transitions.first() {
                    if !t.events.is_empty() || t.cond.is_some() || t.targets.is_empty() {
                        push(
                            ViolationKind::IllegalHistoryTransition,
                            NodeRef::Transition {
                                source: s.id.clone(),
                                index: 0,
                            },
                            t.span,
                        );
                    }
                    if let Some(p) = flat.parent[i] {
                        for target in &t.targets {
                            if let Some(&ti) = flat.by_id.get(target.as_str())
                                && !flat.is_descendant(ti, p)
                            {
                                push(
                                    ViolationKind::IllegalHistoryTarget {
                                        target: target.clone(),
                                    },
                                    NodeRef::Transition {
                                        source: s.id.clone(),
                                        index: 0,
                                    },
                                    t.span,
                                );
                            }
                        }
                    }
                }
                if !s.children.is_empty()
                    || !s.onentry.is_empty()
                    || !s.onexit.is_empty()
                    || !s.invokes.is_empty()
                    || !s.data.is_empty()
                {
                    push(ViolationKind::HistoryContent, sid(), s.span);
                }
            }
            StateKind::Final => {
                let mut what = Vec::new();
                if !s.children.is_empty() {
                    what.push("child states");
                }
                if !s.transitions.is_empty() {
                    what.push("transitions");
                }
                if !s.invokes.is_empty() {
                    what.push("invokes");
                }
                if !s.data.is_empty() {
                    what.push("data");
                }
                if !what.is_empty() {
                    push(
                        ViolationKind::FinalContent {
                            what: what.join(", "),
                        },
                        sid(),
                        s.span,
                    );
                }
            }
            StateKind::Parallel | StateKind::Compound
                if !s.children.is_empty()
                    && s.children
                        .iter()
                        .all(|c| matches!(c.element, StateElement::History(_))) =>
            {
                // A `<state>` whose only children are history states is atomic in W3C SCXML
                // but compound by `State::kind`; such charts are rejected rather than run
                // under either reading.
                let k = if kind == StateKind::Parallel {
                    ViolationKind::ParallelHistoryOnly
                } else {
                    ViolationKind::StateHistoryOnly
                };
                push(k, sid(), s.span);
            }
            _ => {}
        }
        if s.donedata.is_some() && kind != StateKind::Final {
            push(ViolationKind::DoneDataOutsideFinal, sid(), s.span);
        }
        if let Some(dd) = &s.donedata {
            if dd.content.is_some() && !dd.params.is_empty() {
                push(conflict("donedata", "content, param"), sid(), dd.span);
            }
            check_params(&dd.params, &s.id, &mut push);
        }

        // Executable content and invokes.
        for b in s.onentry.iter().chain(s.onexit.iter()) {
            check_block(b, &s.id, &mut push);
        }
        for (ii, inv) in s.invokes.iter().enumerate() {
            let inode = || NodeRef::Invoke {
                state: s.id.clone(),
                index: ii,
            };
            let pairs = [
                (
                    inv.type_.is_some() && inv.typeexpr.is_some(),
                    "type, typeexpr",
                ),
                (inv.src.is_some() && inv.srcexpr.is_some(), "src, srcexpr"),
                (
                    inv.id.is_some() && inv.idlocation.is_some(),
                    "id, idlocation",
                ),
                // A resolved `src` keeps the reference and the document it resolved to as an
                // inline `<content>` body, so only `srcexpr`, or `src` with a `<content expr>`,
                // conflicts with content.
                (
                    inv.content.as_ref().is_some_and(|c| {
                        inv.srcexpr.is_some() || (inv.src.is_some() && c.expr.is_some())
                    }),
                    "content, src",
                ),
                (
                    !inv.params.is_empty() && !inv.namelist.is_empty(),
                    "param, namelist",
                ),
            ];
            for (bad, attrs) in pairs {
                if bad {
                    push(conflict("invoke", attrs), inode(), inv.span);
                }
            }
            check_params(&inv.params, &s.id, &mut push);
            check_block(&inv.finalize, &s.id, &mut push);
        }
    }

    if v.is_empty() { Ok(()) } else { Err(v) }
}

fn conflict(element: &str, attributes: &str) -> ViolationKind {
    ViolationKind::ConflictingAttributes {
        element: element.to_string(),
        attributes: attributes.to_string(),
    }
}

/// The history states the default entry of history state `h` targets: its default transition's
/// history targets, or, without one, the history targets of its parent's `initial` attribute or
/// `<initial>` transition (which is what compilation synthesizes as the default).
fn history_default_edges(flat: &Flat<'_>, h: usize) -> Vec<usize> {
    let st = flat.states[h];
    let targets: Vec<&str> = match (st.transitions.first(), flat.parent[h]) {
        (Some(t), _) => t.targets.iter().map(String::as_str).collect(),
        (None, Some(p)) => resolve_initial(flat.states[p]).targets,
        (None, None) => Vec::new(),
    };
    targets
        .iter()
        .filter_map(|t| flat.by_id.get(t).copied())
        .filter(|&i| matches!(flat.states[i].element, StateElement::History(_)))
        .collect()
}

/// Default history entry must terminate: no history state may reach itself through
/// [`history_default_edges`]. Without this rule entering such a history state would recurse
/// forever.
fn check_history_defaults(flat: &Flat<'_>, push: &mut impl FnMut(ViolationKind, NodeRef, Span)) {
    let histories: Vec<usize> = (0..flat.states.len())
        .filter(|&i| matches!(flat.states[i].element, StateElement::History(_)))
        .collect();
    for &h in &histories {
        let mut seen = alloc::vec![false; flat.states.len()];
        let mut stack = history_default_edges(flat, h);
        let mut cyclic = false;
        while let Some(x) = stack.pop() {
            if x == h {
                cyclic = true;
                break;
            }
            if !core::mem::replace(&mut seen[x], true) {
                stack.extend(history_default_edges(flat, x));
            }
        }
        if cyclic {
            let id = flat.states[h].id.clone();
            push(
                ViolationKind::HistoryDefaultCycle {
                    history: id.clone(),
                },
                NodeRef::State { id },
                flat.states[h].span,
            );
        }
    }
}

/// Targets of one transition must be able to be active together: no target may be an ancestor of
/// another, and every pair must meet below a parallel state. A history target stands for the
/// whole content of its parent, so it takes its parent's position: it excludes its siblings,
/// their descendants and other history states of the same parent.
fn check_target_set(
    flat: &Flat<'_>,
    targets: &[String],
    node: NodeRef,
    span: Span,
    push: &mut impl FnMut(ViolationKind, NodeRef, Span),
) {
    let idx: Vec<(usize, &String)> = targets
        .iter()
        .filter_map(|t| flat.by_id.get(t.as_str()).map(|i| (*i, t)))
        .filter_map(|(i, t)| {
            if matches!(flat.states[i].element, StateElement::History(_)) {
                // A history state at the root is reported on its own.
                flat.parent[i].map(|p| (p, t))
            } else {
                Some((i, t))
            }
        })
        .collect();
    for a in 0..idx.len() {
        for b in (a + 1)..idx.len() {
            let (x, xs) = idx[a];
            let (y, ys) = idx[b];
            let ok = if x == y || flat.is_descendant(x, y) || flat.is_descendant(y, x) {
                false
            } else {
                // Lowest common proper ancestor must be a parallel state.
                let mut anc = flat.parent[x];
                loop {
                    match anc {
                        Some(p) if flat.is_descendant(y, p) => {
                            break flat.states[p].kind() == StateKind::Parallel;
                        }
                        Some(p) => anc = flat.parent[p],
                        None => break false,
                    }
                }
            };
            if !ok {
                push(
                    ViolationKind::IllegalTargetSet {
                        first: xs.clone(),
                        second: ys.clone(),
                    },
                    node.clone(),
                    span,
                );
            }
        }
    }
}

fn check_params(
    params: &[Param],
    state: &str,
    push: &mut impl FnMut(ViolationKind, NodeRef, Span),
) {
    for p in params {
        if p.expr.is_some() == p.location.is_some() {
            push(
                conflict("param", "expr, location"),
                NodeRef::Exec {
                    state: state.to_string(),
                },
                p.span,
            );
        }
    }
}

fn check_block(block: &Block, state: &str, push: &mut impl FnMut(ViolationKind, NodeRef, Span)) {
    for e in block {
        check_exec(e, state, push);
    }
}

fn check_exec(e: &Exec, state: &str, push: &mut impl FnMut(ViolationKind, NodeRef, Span)) {
    let node = || NodeRef::Exec {
        state: state.to_string(),
    };
    match &e.kind {
        ExecKind::Send(s) => {
            let pairs = [
                (
                    s.event.is_some() && s.eventexpr.is_some(),
                    "event, eventexpr",
                ),
                (
                    s.target.is_some() && s.targetexpr.is_some(),
                    "target, targetexpr",
                ),
                (s.type_.is_some() && s.typeexpr.is_some(), "type, typeexpr"),
                (s.id.is_some() && s.idlocation.is_some(), "id, idlocation"),
                (
                    s.delay.is_some() && s.delayexpr.is_some(),
                    "delay, delayexpr",
                ),
                (
                    s.content.is_some() && (!s.params.is_empty() || !s.namelist.is_empty()),
                    "content, param, namelist",
                ),
            ];
            for (bad, attrs) in pairs {
                if bad {
                    push(conflict("send", attrs), node(), e.span);
                }
            }
            check_params(&s.params, state, push);
        }
        ExecKind::Cancel { sendid, sendidexpr } if sendid.is_some() == sendidexpr.is_some() => {
            push(conflict("cancel", "sendid, sendidexpr"), node(), e.span);
        }
        ExecKind::Assign { expr, content, .. } if expr.is_some() && content.is_some() => {
            push(conflict("assign", "expr, content"), node(), e.span);
        }
        ExecKind::If { branches } => {
            for (bi, br) in branches.iter().enumerate() {
                let is_else = br.cond.is_none();
                if is_else && bi + 1 != branches.len() {
                    push(
                        conflict("if", &format!("else before branch {}", bi + 1)),
                        node(),
                        br.span,
                    );
                }
                if bi == 0 && is_else {
                    push(conflict("if", "cond"), node(), br.span);
                }
                check_block(&br.body, state, push);
            }
        }
        ExecKind::Foreach { body, item, .. } => {
            if item.is_empty() {
                push(conflict("foreach", "item"), node(), e.span);
            }
            check_block(body, state, push);
        }
        _ => {}
    }
}
