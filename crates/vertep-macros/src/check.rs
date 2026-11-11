//! Compile-time checks: well-formedness and type checking through `vertep-core`, then
//! reachability and event coverage. Every finding is located at the offending tokens.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::Span;
use vertep_core::ir::{self, Chart, State, StateKind};
use vertep_core::types::decl::suggest;
use vertep_core::types::events::descriptor_matches;
use vertep_core::types::{BasicTyper, ChartTypes, Declarations, check_chart};
use vertep_core::validate::{
    NodeRef, ViolationKind, resolve_initial, resolve_root_initial, validate,
};

use crate::lower::SpanTable;

/// Where findings go: the tokens of a `statechart!` invocation, or the call site of an
/// `include_chart!` with the file position in the message.
pub enum Locator<'a> {
    Tokens(&'a SpanTable),
    File { path: String, call: Span },
}

impl Locator<'_> {
    pub fn span(&self, s: ir::Span) -> Span {
        match self {
            Locator::Tokens(t) => t.token(s).unwrap_or_else(Span::call_site),
            Locator::File { call, .. } => *call,
        }
    }

    pub fn message(&self, s: ir::Span, msg: &str) -> String {
        match self {
            Locator::Tokens(_) => msg.to_string(),
            Locator::File { path, .. } if s.is_known() => {
                format!("{path}:{}:{}: {msg}", s.line, s.column)
            }
            Locator::File { path, .. } => format!("{path}: {msg}"),
        }
    }

    fn error(&self, s: ir::Span, msg: &str) -> syn::Error {
        syn::Error::new(self.span(s), self.message(s, msg))
    }

    fn table(&self) -> Option<&SpanTable> {
        match self {
            Locator::Tokens(t) => Some(t),
            Locator::File { .. } => None,
        }
    }
}

/// Everything the checks found.
#[derive(Default)]
pub struct Report {
    pub errors: Vec<syn::Error>,
    pub warnings: Vec<(Span, String)>,
    pub types: Option<ChartTypes>,
}

/// How reachability and coverage findings are reported: warnings by default, errors with
/// `#[deny(...)]`, silent with `#[allow(...)]`.
// @tg: implements:: [[openspec:rust-dsl#Compile-Time Reachability and Coverage Diagnostics]]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Allow,
    Warn,
    Deny,
}

/// Run every check: declarations, well-formedness (each violation at its tokens, unknown
/// targets with a suggestion, duplicate ids at every declaration), types, reachability and
/// coverage.
// @tg: implements:: [[openspec:rust-dsl#Compile-Time Structural Checks]]
pub fn check(
    chart: &Chart,
    decls: Option<&Declarations>,
    loc: &Locator<'_>,
    unreachable_level: Level,
    unhandled_level: Level,
) -> Report {
    let deny_unreachable = unreachable_level == Level::Deny;
    let deny_unhandled = unhandled_level == Level::Deny;
    let mut r = Report::default();

    // Declarations, located per declaration when the macro lowered them.
    let mut decl_errors = false;
    if let Some(d) = decls {
        for e in d.validate() {
            decl_errors = true;
            r.errors.push(loc.error(e.span, &e.to_string()));
        }
    }

    // Well-formedness.
    let structural = match validate(chart) {
        Ok(()) => true,
        Err(violations) => {
            let ids = all_ids(chart);
            for v in &violations {
                structural_error(&mut r, chart, loc, &ids, v);
            }
            false
        }
    };

    // Types.
    if !decl_errors {
        match check_chart(chart, &BasicTyper::for_datamodel("rust")) {
            Ok(t) => r.types = Some(t),
            Err(diags) => {
                for d in diags {
                    r.errors.push(loc.error(
                        d.span,
                        &format!("type error at {}: {}", d.location, d.message),
                    ));
                }
            }
        }
    }

    if !structural {
        return r;
    }

    // Reachability and coverage.
    let ids: BTreeSet<String> = reachable(chart);
    let mut unreachable = Vec::new();
    walk(&chart.states, &mut |s| {
        if !matches!(s.kind(), StateKind::History(_)) && !ids.contains(&s.id) {
            unreachable.push((s.id.clone(), s.span));
        }
    });
    if unreachable_level == Level::Allow {
        unreachable.clear();
    }
    for (id, span) in unreachable {
        let msg = format!(
            "state `{id}` is unreachable from the initial configuration{}",
            if deny_unreachable {
                ""
            } else {
                " (`#[allow(unreachable_states)]` silences this)"
            }
        );
        if deny_unreachable {
            r.errors.push(loc.error(span, &msg));
        } else {
            r.warnings.push((loc.span(span), loc.message(span, &msg)));
        }
    }
    if let Some(types) = r.types.as_ref().filter(|_| unhandled_level != Level::Allow) {
        let descriptors = all_descriptors(chart);
        for (name, _) in types.events.iter() {
            if descriptors.iter().any(|d| descriptor_matches(d, name)) {
                continue;
            }
            let span = loc
                .table()
                .and_then(|t| t.events.get(name).copied())
                .unwrap_or_else(|| loc.span(chart.span));
            let msg = loc.message(
                ir::Span::NONE,
                &format!(
                    "event `{name}` is declared but no transition handles it{}",
                    if deny_unhandled {
                        ""
                    } else {
                        " (`#[allow(unhandled_events)]` silences this)"
                    }
                ),
            );
            if deny_unhandled {
                r.errors.push(syn::Error::new(span, msg));
            } else {
                r.warnings.push((span, msg));
            }
        }
    }
    r
}

fn walk<'c>(states: &'c [State], f: &mut dyn FnMut(&'c State)) {
    for s in states {
        f(s);
        walk(&s.children, f);
    }
}

fn all_ids(chart: &Chart) -> Vec<String> {
    let mut ids = Vec::new();
    walk(&chart.states, &mut |s| ids.push(s.id.clone()));
    ids
}

fn all_descriptors(chart: &Chart) -> Vec<String> {
    let mut out = Vec::new();
    walk(&chart.states, &mut |s| {
        for t in &s.transitions {
            out.extend(t.events.iter().cloned());
        }
    });
    out
}

fn owner_span(chart: &Chart, node: &NodeRef) -> Option<ir::Span> {
    let find = |id: &str| {
        let mut found = None;
        walk(&chart.states, &mut |s| {
            if found.is_none() && s.id == id {
                found = Some(s);
            }
        });
        found
    };
    match node {
        NodeRef::Root => Some(chart.span),
        NodeRef::Transition { source, index } => {
            find(source).and_then(|s| s.transitions.get(*index).map(|t| t.span))
        }
        NodeRef::Initial { state } | NodeRef::State { id: state } => find(state).map(|s| s.span),
        _ => None,
    }
}

fn structural_error(
    r: &mut Report,
    chart: &Chart,
    loc: &Locator<'_>,
    ids: &[String],
    v: &vertep_core::validate::Violation,
) {
    let msg = match &v.kind {
        ViolationKind::UnknownTarget { target } => {
            let mut m = format!("unknown target state `{target}`");
            if let Some(s) = suggest(target, ids.iter().map(String::as_str)) {
                m.push_str(&format!("; did you mean `{s}`?"));
            }
            // Point at the target token when the macro lowered it.
            let token = loc.table().and_then(|t| {
                owner_span(chart, &v.node)
                    .and_then(|s| t.targets.get(&s.line))
                    .and_then(|ts| ts.iter().find(|n| n.text == *target))
                    .map(|n| n.span)
            });
            if let Some(span) = token {
                r.errors.push(syn::Error::new(span, m));
                return;
            }
            m
        }
        ViolationKind::DuplicateStateId { id } => {
            let m = format!("state id `{id}` is declared more than once");
            if let Some(spans) = loc.table().and_then(|t| t.state_decls.get(id)) {
                for s in spans {
                    r.errors.push(syn::Error::new(*s, &m));
                }
                return;
            }
            m
        }
        ViolationKind::DuplicateDataId { id } => {
            format!("data item `{id}` is declared more than once")
        }
        ViolationKind::EmptyChart => "the chart declares no states".to_string(),
        ViolationKind::MissingId => "a state has no identifier".to_string(),
        ViolationKind::IllegalInitialTarget { target } => {
            format!("initial target `{target}` is not a descendant of its state")
        }
        ViolationKind::InitialAttributeAndElement => {
            "a state has both an initial attribute and an <initial> element".to_string()
        }
        ViolationKind::InitialOnNonCompound => {
            "`initial` is only allowed on a state with child states".to_string()
        }
        ViolationKind::IllegalInitialTransition => {
            "an initial transition has no event or guard and has a target".to_string()
        }
        ViolationKind::HistoryTransitionCount => {
            "a history state has at most one default transition".to_string()
        }
        ViolationKind::IllegalHistoryTransition => {
            "a history default transition has no event or guard and has a target".to_string()
        }
        ViolationKind::IllegalHistoryTarget { target } => {
            format!("history default target `{target}` is outside the history's parent")
        }
        ViolationKind::HistoryAtRoot => {
            "a history state must be inside a compound or parallel state".to_string()
        }
        ViolationKind::HistoryContent => {
            "a history state contains only its default transition".to_string()
        }
        ViolationKind::FinalContent { what } => {
            format!("a final state cannot contain {what}")
        }
        ViolationKind::DoneDataOutsideFinal => "donedata belongs on final states".to_string(),
        ViolationKind::ParallelHistoryOnly | ViolationKind::StateHistoryOnly => {
            "a state needs a child state that is not a history state".to_string()
        }
        ViolationKind::IllegalTargetSet { first, second } => {
            format!("targets `{first}` and `{second}` cannot be active together")
        }
        ViolationKind::HistoryDefaultCycle { history } => {
            format!("the default entry of history `{history}` never reaches a state")
        }
        ViolationKind::ConflictingAttributes {
            element,
            attributes,
        } => format!("{element}: conflicting or missing {attributes}"),
    };
    let span = owner_span(chart, &v.node).unwrap_or(v.span);
    r.errors
        .push(loc.error(if v.span.is_known() { v.span } else { span }, &msg));
}

/// The states some run can make active: from the initial configuration, through every
/// transition (with or without events and guards) of an active state.
// @tg: implements:: [[openspec:rust-dsl#Compile-Time Reachability and Coverage Diagnostics]]
fn reachable(chart: &Chart) -> BTreeSet<String> {
    struct Graph<'c> {
        states: Vec<&'c State>,
        parent: Vec<Option<usize>>,
        by_id: BTreeMap<&'c str, usize>,
    }
    fn index<'c>(g: &mut Graph<'c>, s: &'c State, parent: Option<usize>) {
        let i = g.states.len();
        g.states.push(s);
        g.parent.push(parent);
        g.by_id.entry(s.id.as_str()).or_insert(i);
        for c in &s.children {
            index(g, c, Some(i));
        }
    }
    let mut g = Graph {
        states: Vec::new(),
        parent: Vec::new(),
        by_id: BTreeMap::new(),
    };
    for s in &chart.states {
        index(&mut g, s, None);
    }
    let mut active = vec![false; g.states.len()];
    let mut entered = vec![false; g.states.len()];
    let mut work = Vec::new();

    fn enter(
        g: &Graph<'_>,
        i: usize,
        active: &mut [bool],
        entered: &mut [bool],
        work: &mut Vec<usize>,
    ) {
        if entered[i] {
            return;
        }
        entered[i] = true;
        // The state and its ancestors become active; parallel ancestors enter their other
        // regions.
        let mut child = i;
        let mut cur = Some(i);
        while let Some(c) = cur {
            if !active[c] {
                active[c] = true;
                work.push(c);
            }
            if c != i && g.states[c].kind() == StateKind::Parallel {
                for (k, _) in g.states.iter().enumerate() {
                    if g.parent[k] == Some(c)
                        && k != child
                        && !matches!(g.states[k].kind(), StateKind::History(_))
                    {
                        enter(g, k, active, entered, work);
                    }
                }
            }
            child = c;
            cur = g.parent[c];
        }
        let s = g.states[i];
        let targets: Vec<&str> = match s.kind() {
            StateKind::Compound => resolve_initial(s).targets,
            StateKind::Parallel => s
                .children
                .iter()
                .filter(|c| !matches!(c.kind(), StateKind::History(_)))
                .map(|c| c.id.as_str())
                .collect(),
            StateKind::History(_) => {
                let t: Vec<&str> = s
                    .transitions
                    .iter()
                    .flat_map(|t| t.targets.iter().map(String::as_str))
                    .collect();
                if t.is_empty() {
                    // Default to the parent's default entry.
                    match g.parent[i] {
                        Some(p) => {
                            enter(g, p, active, entered, work);
                            Vec::new()
                        }
                        None => Vec::new(),
                    }
                } else {
                    t
                }
            }
            _ => Vec::new(),
        };
        for t in targets {
            if let Some(&k) = g.by_id.get(t) {
                enter(g, k, active, entered, work);
            }
        }
    }

    for t in resolve_root_initial(chart).targets {
        if let Some(&k) = g.by_id.get(t) {
            enter(&g, k, &mut active, &mut entered, &mut work);
        }
    }
    while let Some(i) = work.pop() {
        let s = g.states[i];
        if matches!(s.kind(), StateKind::History(_)) {
            continue;
        }
        for t in &s.transitions {
            for target in &t.targets {
                if let Some(&k) = g.by_id.get(target.as_str()) {
                    enter(&g, k, &mut active, &mut entered, &mut work);
                }
            }
        }
    }
    g.states
        .iter()
        .enumerate()
        .filter(|(i, _)| active[*i])
        .map(|(_, s)| s.id.clone())
        .collect()
}
