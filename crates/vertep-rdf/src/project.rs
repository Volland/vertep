//! The projection of a chart into an RDF 1.2 graph under the `vt:` chart ontology.
//!
//! # Node IRIs
//!
//! Every node is named `<base>#<fragment>`; the projection uses no blank nodes. The fragment of
//! the chart node is `chart`; a state's is `s.` followed by its percent-encoded identifier; any
//! other node's is its owner's fragment, `:`, and a step naming its role and position:
//!
//! | step | node |
//! |---|---|
//! | `ia<j>` | `j`-th member of the owner's `initial` attribute |
//! | `init`, `init:t` | the `<initial>` element and its transition |
//! | `t<i>` | `i`-th transition |
//! | `ev<j>`, `tg<j>` | `j`-th event descriptor and target of a transition |
//! | `en<i>`, `ex<i>` | `i`-th `<onentry>` and `<onexit>` handler |
//! | `a<k>` | `k`-th action of a transition, handler, branch or `<foreach>` |
//! | `b<j>` | `j`-th branch of an `<if>` |
//! | `d<i>`, `scr` | `i`-th data item, top-level script |
//! | `inv<i>`, `f<k>` | `i`-th invoke, `k`-th action of its `<finalize>` |
//! | `dd`, `p<j>`, `c`, `nl<j>` | donedata, `j`-th param, content, `j`-th namelist name |
//! | `x<i>` | `i`-th extension node |
//!
//! State IRIs depend only on the identifier, so a target reference points at the state IRI its
//! identifier mints whether or not such a state exists; the shapes then reject a dangling one.
//!
//! # Derived triples
//!
//! The projection also materializes, for the SHACL Core shapes (design D9): `vt:ancestor` from
//! each state to every proper ancestor state and to the chart node; `vt:source` from each
//! transition to its source state; and `vt:scope` from each target reference to the node whose
//! proper descendants it may name (the owning state for `initial` attributes and `<initial>`
//! transitions, the history's parent for history transitions, the chart otherwise).
//!
//! Finally, for every target list (a transition's targets, an `initial` attribute) it mints
//! *selection nodes* `<list>:sel:<X>`, one per node `X` on the ancestor chain of a listed
//! state `t` (for a history state, its parent, whose content it stands for), with `vt:choice`
//! naming the branch `t` takes at `X`: the child of `X`
//! towards `t` when `X` is not parallel, `X` itself when it is parallel, and the target
//! reference when `X` is `t`. A target list can be active together exactly when no selection
//! node has two choices, which SHACL Core states as `sh:maxCount 1` (the chart model's legal
//! target set rule, which is pairwise and otherwise needs SPARQL). And from every history
//! state it materializes `vt:historyDefault` to the history states its default entry targets,
//! so that the shapes can reject default-entry cycles.

use std::collections::HashMap;

use oxrdf::{Graph, Literal, NamedNode, NamedNodeRef, Term, TripleRef};
use vertep_core::ir::{
    Binding, Block, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, Invoke, Param, Script, State, StateElement, StateKind, Transition, TransitionType,
};

use crate::iri::{self, ChartVersion, PLACEHOLDER_BASE};
use crate::vocab::{self as vt, expr_datatype};

/// A chart's projection, named by its version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedChart {
    /// The chart version; also the base of every node IRI.
    pub version: ChartVersion,
    /// The projection, minted under the version IRI.
    pub graph: Graph,
}

/// Project a chart, compute its version and mint its node IRIs under it.
// @tg: implements:: [[openspec:chart-rdf#IR To RDF Projection]]
pub fn project(chart: &Chart) -> ProjectedChart {
    let version = crate::hash::chart_version(chart);
    let graph = project_with_base(chart, version.as_str());
    ProjectedChart { version, graph }
}

/// Project a chart with node IRIs minted under `base`.
///
/// [`project`] calls this with the version IRI; hashing calls it with [`PLACEHOLDER_BASE`].
// @tg: implements:: [[openspec:chart-rdf#Explicit Ordering]]
// @tg: implements:: [[openspec:chart-rdf#Executable Content As Literals]]
pub fn project_with_base(chart: &Chart, base: &str) -> Graph {
    let mut p = Projector {
        base,
        graph: Graph::new(),
        expr_dt: NamedNode::new_unchecked(expr_datatype(&chart.datamodel)),
        chains: HashMap::new(),
        histories: HashMap::new(),
    };
    let mut chain = vec![(String::from(iri::CHART_FRAGMENT), false)];
    for s in &chart.states {
        p.index(s, &mut chain);
    }
    p.chart(chart);
    p.history_defaults(&chart.states, None);
    p.graph
}

/// Project a chart under [`PLACEHOLDER_BASE`]: the graph whose canonical form is hashed.
pub fn project_unversioned(chart: &Chart) -> Graph {
    project_with_base(chart, PLACEHOLDER_BASE)
}

/// The fragment of the state with this identifier.
pub fn state_fragment(id: &str) -> String {
    let mut f = String::from("s.");
    iri::percent_encode_into(id, &mut f);
    f
}

/// One link of an ancestor chain: a node's fragment and whether it is a parallel state.
type Link = (String, bool);

struct Projector<'a> {
    base: &'a str,
    graph: Graph,
    expr_dt: NamedNode,
    /// For each state identifier (first in document order) that names a non-history state:
    /// the chain from the chart node down to the state itself.
    chains: HashMap<&'a str, Vec<Link>>,
    /// For each state identifier (first in document order): whether it names a history state.
    histories: HashMap<&'a str, bool>,
}

impl<'a> Projector<'a> {
    fn index(&mut self, state: &'a State, chain: &mut Vec<Link>) {
        chain.push((
            state_fragment(&state.id),
            state.element == StateElement::Parallel,
        ));
        let entry = if !matches!(state.element, StateElement::History(_)) {
            chain.clone()
        } else if chain.len() > 2 {
            // A history target stands for its parent's content: it takes its parent's place.
            chain[..chain.len() - 1].to_vec()
        } else {
            // A history state at the root is reported on its own; it still shadows later
            // states with the same identifier.
            Vec::new()
        };
        self.chains.entry(state.id.as_str()).or_insert(entry);
        self.histories
            .entry(state.id.as_str())
            .or_insert(matches!(state.element, StateElement::History(_)));
        for c in &state.children {
            self.index(c, chain);
        }
        chain.pop();
    }

    /// The `vt:historyDefault` edges: from each history state to the history states its default
    /// entry targets, by its default transition or, without one, by its parent's initial
    /// targets (`initial` attribute, else `<initial>` transition, of a compound parent).
    fn history_defaults(&mut self, states: &'a [State], parent: Option<&'a State>) {
        for s in states {
            if matches!(s.element, StateElement::History(_)) {
                let targets: &[String] = match (s.transitions.first(), parent) {
                    (Some(t), _) => &t.targets,
                    (None, Some(p)) if p.kind() == StateKind::Compound => {
                        match (&p.initial, &p.initial_element) {
                            (ids, _) if !ids.is_empty() => ids,
                            (_, Some(init)) => &init.transition.targets,
                            _ => &[],
                        }
                    }
                    _ => &[],
                };
                let from = self.iri(&state_fragment(&s.id));
                for t in targets {
                    if self.histories.get(t.as_str()) == Some(&true) {
                        let to = self.iri(&state_fragment(t));
                        self.add(&from, vt::HISTORY_DEFAULT, to);
                    }
                }
            }
            self.history_defaults(&s.children, Some(s));
        }
    }

    fn iri(&self, fragment: &str) -> NamedNode {
        NamedNode::new_unchecked(iri::node_iri(self.base, fragment))
    }

    fn add(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, o: impl Into<Term>) {
        let o = o.into();
        self.graph.insert(TripleRef::new(s, p, &o));
    }

    fn ty(&mut self, s: &NamedNode, class: NamedNodeRef<'_>) {
        self.add(
            s,
            NamedNodeRef::new_unchecked(vt::RDF_TYPE),
            class.into_owned(),
        );
    }

    fn string(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, v: &str) {
        self.add(s, p, Literal::new_simple_literal(v));
    }

    fn opt_string(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, v: &Option<String>) {
        if let Some(v) = v {
            self.string(s, p, v);
        }
    }

    fn boolean(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, v: bool) {
        self.add(s, p, Literal::from(v));
    }

    fn integer(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, v: u64) {
        self.add(
            s,
            p,
            Literal::new_typed_literal(v.to_string(), NamedNodeRef::new_unchecked(vt::XSD_INTEGER)),
        );
    }

    fn order(&mut self, s: &NamedNode, i: usize) {
        self.integer(s, vt::ORDER, i as u64);
    }

    fn expr(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, v: &str) {
        let lit = Literal::new_typed_literal(v, self.expr_dt.clone());
        self.add(s, p, lit);
    }

    fn opt_expr(&mut self, s: &NamedNode, p: NamedNodeRef<'_>, v: &Option<String>) {
        if let Some(v) = v {
            self.expr(s, p, v);
        }
    }

    /// Create a node, link it from its owner and give it its classes.
    fn node(
        &mut self,
        owner: &NamedNode,
        prop: NamedNodeRef<'_>,
        fragment: &str,
        classes: &[NamedNodeRef<'_>],
    ) -> NamedNode {
        let n = self.iri(fragment);
        self.add(owner, prop, n.clone());
        for c in classes {
            self.ty(&n, *c);
        }
        n
    }

    fn chart(&mut self, chart: &Chart) {
        let frag = iri::CHART_FRAGMENT;
        let c = self.iri(frag);
        self.ty(&c, vt::CHART);
        self.opt_string(&c, vt::NAME, &chart.name);
        self.string(&c, vt::DATAMODEL, &chart.datamodel);
        self.boolean(&c, vt::DATAMODEL_EXPLICIT, chart.datamodel_explicit);
        let binding = match chart.binding {
            Binding::Early => "early",
            Binding::Late => "late",
        };
        self.string(&c, vt::BINDING, binding);
        self.boolean(&c, vt::BINDING_EXPLICIT, chart.binding_explicit);
        let list = format!("{frag}:ia");
        self.targets(&c, vt::INITIAL_REF, &list, "", &chart.initial, &c);
        for (i, d) in chart.data.iter().enumerate() {
            self.data_item(&c, frag, i, d);
        }
        if let Some(s) = &chart.script {
            let n = self.node(&c, vt::SCRIPT_P, &format!("{frag}:scr"), &[vt::SCRIPT]);
            self.script_fields(&n, s);
        }
        let mut ancestors = vec![c.clone()];
        for (i, s) in chart.states.iter().enumerate() {
            self.state(s, i, &c, &mut ancestors);
        }
        self.extensions(&c, frag, &chart.extensions);
    }

    /// Project a target list: one `vt:TargetRef` per member and the selection nodes.
    fn targets(
        &mut self,
        owner: &NamedNode,
        prop: NamedNodeRef<'_>,
        list: &str,
        ref_step: &str,
        ids: &[String],
        scope: &NamedNode,
    ) {
        for (j, id) in ids.iter().enumerate() {
            let r = self.node(
                owner,
                prop,
                &format!("{list}{ref_step}{j}"),
                &[vt::TARGET_REF],
            );
            self.order(&r, j);
            let target = self.iri(&state_fragment(id));
            self.add(&r, vt::STATE_P, target);
            self.add(&r, vt::SCOPE, scope.clone());
            let Some(chain) = self.chains.get(id.as_str()).cloned() else {
                continue;
            };
            for (i, (x, parallel)) in chain.iter().enumerate() {
                let sel = self.iri(&format!("{list}:sel:{x}"));
                let choice = match chain.get(i + 1) {
                    None => r.clone(),
                    Some(_) if *parallel => self.iri(x),
                    Some((c, _)) => self.iri(c),
                };
                self.add(&sel, vt::CHOICE, choice);
            }
        }
    }

    fn tokens(
        &mut self,
        owner: &NamedNode,
        prop: NamedNodeRef<'_>,
        prefix: &str,
        values: &[String],
    ) {
        for (j, v) in values.iter().enumerate() {
            let t = self.node(owner, prop, &format!("{prefix}{j}"), &[vt::TOKEN]);
            self.order(&t, j);
            self.string(&t, vt::VALUE, v);
        }
    }

    fn state(
        &mut self,
        state: &State,
        order: usize,
        parent: &NamedNode,
        ancestors: &mut Vec<NamedNode>,
    ) {
        let frag = state_fragment(&state.id);
        let element = match state.element {
            StateElement::State => vt::STATE,
            StateElement::Parallel => vt::PARALLEL,
            StateElement::Final => vt::FINAL,
            StateElement::History(_) => vt::HISTORY,
        };
        let s = self.node(parent, vt::CHILD, &frag, &[vt::STATE_NODE, element]);
        if let StateElement::History(h) = state.element {
            let h = match h {
                HistoryType::Shallow => "shallow",
                HistoryType::Deep => "deep",
            };
            self.string(&s, vt::HISTORY_TYPE, h);
        }
        self.string(&s, vt::ID, &state.id);
        self.boolean(&s, vt::ID_GENERATED, state.id_generated);
        self.order(&s, order);
        for a in ancestors.iter() {
            self.add(&s, vt::ANCESTOR, a.clone());
        }
        self.targets(
            &s,
            vt::INITIAL_REF,
            &format!("{frag}:ia"),
            "",
            &state.initial,
            &s,
        );
        if let Some(init) = &state.initial_element {
            let ifrag = format!("{frag}:init");
            let n = self.node(&s, vt::INITIAL_ELEMENT, &ifrag, &[vt::INITIAL]);
            let tfrag = format!("{ifrag}:t");
            self.transition(
                &init.transition,
                &n,
                vt::INITIAL_TRANSITION,
                &tfrag,
                0,
                &s,
                &s,
            );
            self.extensions(&n, &ifrag, &init.extensions);
        }
        let chart = ancestors[0].clone();
        let scope = if matches!(state.element, StateElement::History(_)) {
            parent.clone()
        } else {
            chart
        };
        for (i, t) in state.transitions.iter().enumerate() {
            let tfrag = format!("{frag}:t{i}");
            self.transition(t, &s, vt::TRANSITION_P, &tfrag, i, &s, &scope);
        }
        for (i, b) in state.onentry.iter().enumerate() {
            let hfrag = format!("{frag}:en{i}");
            let h = self.node(&s, vt::ONENTRY, &hfrag, &[vt::ON_ENTRY]);
            self.order(&h, i);
            self.block(&h, vt::ACTION, &format!("{hfrag}:a"), b);
        }
        for (i, b) in state.onexit.iter().enumerate() {
            let hfrag = format!("{frag}:ex{i}");
            let h = self.node(&s, vt::ONEXIT, &hfrag, &[vt::ON_EXIT]);
            self.order(&h, i);
            self.block(&h, vt::ACTION, &format!("{hfrag}:a"), b);
        }
        for (i, d) in state.data.iter().enumerate() {
            self.data_item(&s, &frag, i, d);
        }
        for (i, inv) in state.invokes.iter().enumerate() {
            self.invoke(&s, &format!("{frag}:inv{i}"), i, inv);
        }
        if let Some(dd) = &state.donedata {
            self.donedata(&s, &format!("{frag}:dd"), dd);
        }
        self.extensions(&s, &frag, &state.extensions);
        ancestors.push(s.clone());
        for (i, c) in state.children.iter().enumerate() {
            self.state(c, i, &s, ancestors);
        }
        ancestors.pop();
    }

    #[allow(clippy::too_many_arguments)]
    fn transition(
        &mut self,
        t: &Transition,
        owner: &NamedNode,
        prop: NamedNodeRef<'_>,
        frag: &str,
        order: usize,
        source: &NamedNode,
        scope: &NamedNode,
    ) {
        let n = self.node(owner, prop, frag, &[vt::TRANSITION]);
        self.order(&n, order);
        self.add(&n, vt::SOURCE, source.clone());
        self.tokens(&n, vt::EVENT, &format!("{frag}:ev"), &t.events);
        self.opt_expr(&n, vt::COND, &t.cond);
        self.targets(&n, vt::TARGET, frag, ":tg", &t.targets, scope);
        let kind = match t.kind {
            TransitionType::External => "external",
            TransitionType::Internal => "internal",
        };
        self.string(&n, vt::TRANSITION_TYPE, kind);
        self.boolean(&n, vt::TRANSITION_TYPE_EXPLICIT, t.kind_explicit);
        self.block(&n, vt::ACTION, &format!("{frag}:a"), &t.body);
        self.extensions(&n, frag, &t.extensions);
    }

    fn block(&mut self, owner: &NamedNode, prop: NamedNodeRef<'_>, prefix: &str, block: &Block) {
        for (k, e) in block.iter().enumerate() {
            self.exec(e, owner, prop, &format!("{prefix}{k}"), k);
        }
    }

    fn exec(
        &mut self,
        e: &Exec,
        owner: &NamedNode,
        prop: NamedNodeRef<'_>,
        frag: &str,
        order: usize,
    ) {
        let class = match &e.kind {
            ExecKind::Raise { .. } => vt::RAISE,
            ExecKind::Send(_) => vt::SEND,
            ExecKind::Cancel { .. } => vt::CANCEL,
            ExecKind::Log { .. } => vt::LOG,
            ExecKind::Assign { .. } => vt::ASSIGN,
            ExecKind::Script(_) => vt::SCRIPT,
            ExecKind::If { .. } => vt::IF,
            ExecKind::Foreach { .. } => vt::FOREACH,
            ExecKind::Extension(_) => vt::EXTENSION_CONTENT,
        };
        let n = self.node(owner, prop, frag, &[vt::EXECUTABLE_CONTENT, class]);
        self.order(&n, order);
        match &e.kind {
            ExecKind::Raise { event } => self.string(&n, vt::EVENT_NAME, event),
            ExecKind::Send(s) => {
                self.opt_string(&n, vt::EVENT_NAME, &s.event);
                self.opt_expr(&n, vt::EVENTEXPR, &s.eventexpr);
                self.opt_string(&n, vt::SEND_TARGET, &s.target);
                self.opt_expr(&n, vt::TARGETEXPR, &s.targetexpr);
                self.opt_string(&n, vt::TYPE, &s.type_);
                self.opt_expr(&n, vt::TYPEEXPR, &s.typeexpr);
                self.opt_string(&n, vt::SEND_ID, &s.id);
                self.opt_expr(&n, vt::IDLOCATION, &s.idlocation);
                self.opt_string(&n, vt::DELAY, &s.delay);
                self.opt_expr(&n, vt::DELAYEXPR, &s.delayexpr);
                self.tokens(&n, vt::NAMELIST, &format!("{frag}:nl"), &s.namelist);
                self.params(&n, frag, &s.params);
                if let Some(c) = &s.content {
                    self.content(&n, frag, c);
                }
            }
            ExecKind::Cancel { sendid, sendidexpr } => {
                self.opt_string(&n, vt::SEND_ID, sendid);
                self.opt_expr(&n, vt::SENDIDEXPR, sendidexpr);
            }
            ExecKind::Log { label, expr } => {
                self.opt_string(&n, vt::LABEL, label);
                self.opt_expr(&n, vt::EXPR, expr);
            }
            ExecKind::Assign {
                location,
                expr,
                content,
            } => {
                self.expr(&n, vt::LOCATION, location);
                self.opt_expr(&n, vt::EXPR, expr);
                self.opt_string(&n, vt::INLINE_CONTENT, content);
            }
            ExecKind::Script(s) => self.script_fields(&n, s),
            ExecKind::If { branches } => {
                for (j, b) in branches.iter().enumerate() {
                    let bfrag = format!("{frag}:b{j}");
                    let bn = self.node(&n, vt::BRANCH, &bfrag, &[vt::IF_BRANCH]);
                    self.order(&bn, j);
                    self.opt_expr(&bn, vt::COND, &b.cond);
                    self.block(&bn, vt::ACTION, &format!("{bfrag}:a"), &b.body);
                }
            }
            ExecKind::Foreach {
                array,
                item,
                index,
                body,
            } => {
                self.expr(&n, vt::ARRAY, array);
                self.string(&n, vt::ITEM, item);
                self.opt_string(&n, vt::INDEX, index);
                self.block(&n, vt::ACTION, &format!("{frag}:a"), body);
            }
            ExecKind::Extension(x) => self.extension_fields(&n, x),
        }
        self.extensions(&n, frag, &e.extensions);
    }

    fn script_fields(&mut self, n: &NamedNode, s: &Script) {
        self.opt_string(n, vt::SRC, &s.src);
        self.opt_expr(n, vt::SCRIPT_BODY, &s.body);
    }

    fn params(&mut self, owner: &NamedNode, frag: &str, params: &[Param]) {
        for (j, p) in params.iter().enumerate() {
            let n = self.node(owner, vt::PARAM_P, &format!("{frag}:p{j}"), &[vt::PARAM]);
            self.order(&n, j);
            self.string(&n, vt::PARAM_NAME, &p.name);
            self.opt_expr(&n, vt::EXPR, &p.expr);
            self.opt_expr(&n, vt::LOCATION, &p.location);
        }
    }

    fn content(&mut self, owner: &NamedNode, frag: &str, c: &Content) {
        let n = self.node(owner, vt::CONTENT_P, &format!("{frag}:c"), &[vt::CONTENT]);
        self.opt_expr(&n, vt::EXPR, &c.expr);
        self.opt_string(&n, vt::INLINE_CONTENT, &c.body);
    }

    fn data_item(&mut self, owner: &NamedNode, owner_frag: &str, i: usize, d: &Data) {
        let frag = format!("{owner_frag}:d{i}");
        let n = self.node(owner, vt::DATA, &frag, &[vt::DATA_ITEM]);
        self.order(&n, i);
        self.string(&n, vt::DATA_ID, &d.id);
        self.opt_string(&n, vt::SRC, &d.src);
        self.opt_expr(&n, vt::EXPR, &d.expr);
        self.opt_string(&n, vt::INLINE_CONTENT, &d.content);
        self.extensions(&n, &frag, &d.extensions);
    }

    fn donedata(&mut self, owner: &NamedNode, frag: &str, dd: &DoneData) {
        let n = self.node(owner, vt::DONEDATA, frag, &[vt::DONE_DATA]);
        if let Some(c) = &dd.content {
            self.content(&n, frag, c);
        }
        self.params(&n, frag, &dd.params);
    }

    fn invoke(&mut self, owner: &NamedNode, frag: &str, order: usize, inv: &Invoke) {
        let n = self.node(owner, vt::INVOKE_P, frag, &[vt::INVOKE]);
        self.order(&n, order);
        self.opt_string(&n, vt::TYPE, &inv.type_);
        self.opt_expr(&n, vt::TYPEEXPR, &inv.typeexpr);
        self.opt_string(&n, vt::SRC, &inv.src);
        self.opt_expr(&n, vt::SRCEXPR, &inv.srcexpr);
        self.opt_string(&n, vt::INVOKE_ID, &inv.id);
        self.opt_expr(&n, vt::IDLOCATION, &inv.idlocation);
        self.tokens(&n, vt::NAMELIST, &format!("{frag}:nl"), &inv.namelist);
        self.boolean(&n, vt::AUTOFORWARD, inv.autoforward);
        self.params(&n, frag, &inv.params);
        if let Some(c) = &inv.content {
            self.content(&n, frag, c);
        }
        self.block(&n, vt::FINALIZE, &format!("{frag}:f"), &inv.finalize);
        self.extensions(&n, frag, &inv.extensions);
    }

    fn extensions(&mut self, owner: &NamedNode, owner_frag: &str, exts: &[Extension]) {
        for (i, x) in exts.iter().enumerate() {
            let n = self.node(
                owner,
                vt::EXTENSION,
                &format!("{owner_frag}:x{i}"),
                &[vt::EXTENSION_NODE],
            );
            self.order(&n, i);
            self.extension_fields(&n, x);
        }
    }

    fn extension_fields(&mut self, n: &NamedNode, x: &Extension) {
        self.string(n, vt::NAMESPACE, &x.ns);
        self.string(n, vt::LOCAL_NAME, &x.local);
        match &x.value {
            ExtensionValue::Attribute(v) => self.string(n, vt::ATTRIBUTE_VALUE, v),
            ExtensionValue::Element(v) => self.string(n, vt::ELEMENT_XML, v),
        }
        self.integer(n, vt::POSITION, u64::from(x.position));
    }
}
