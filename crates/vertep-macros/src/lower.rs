//! Lowering the `statechart!` surface AST to the chart IR.
//!
//! Every IR node gets a synthetic span `line = k` that indexes the [`SpanTable`], so diagnostics
//! from the core validator and type checker map back to the exact tokens; [`SpanTable::finish`]
//! then replaces the synthetic spans with the tokens' real lines and columns.

use std::collections::BTreeMap;

use proc_macro2::Span;
use vertep_core::ir::{
    self, Binding, Chart, Data, Exec, ExecKind, HistoryType, Invoke, State, StateElement,
    Transition, TransitionType,
};
use vertep_core::types::chart::{set_data_type, set_declarations};
use vertep_core::types::{Declarations, EventDecl, TypeDecl, parse_type};

use crate::conventions::{action_exec, action_iri_for, guard_iri_for};
use crate::parse::{ActionAst, ChartAst, DataAst, GuardAst, Named, StateAst, StateKw, TypeText};

/// Maps synthetic IR spans to token spans.
#[derive(Default)]
pub struct SpanTable {
    spans: Vec<Span>,
    /// Target tokens of transitions and `initial` lists, keyed by the owner's synthetic line.
    pub targets: BTreeMap<u32, Vec<Named>>,
    /// Declaration spans of every state, by identifier.
    pub state_decls: BTreeMap<String, Vec<Span>>,
    /// The token of each declared event.
    pub events: BTreeMap<String, Span>,
    /// The first token naming each guard or action function.
    pub functions: BTreeMap<String, Span>,
}

impl SpanTable {
    /// A synthetic IR span for a token span.
    pub fn mark(&mut self, span: Span) -> ir::Span {
        self.spans.push(span);
        ir::Span::new(self.spans.len() as u32, 0)
    }

    /// The token span of a synthetic IR span.
    pub fn token(&self, span: ir::Span) -> Option<Span> {
        let i = span.line as usize;
        (i > 0).then(|| self.spans.get(i - 1).copied()).flatten()
    }

    /// The real position of a synthetic span.
    pub fn real(&self, span: ir::Span) -> ir::Span {
        match self.token(span) {
            Some(s) => {
                let lc = s.start();
                ir::Span::new(lc.line as u32, lc.column as u32 + 1)
            }
            None => ir::Span::NONE,
        }
    }
}

/// Lowering errors that the core does not see (they are about the surface).
pub struct Lowered {
    pub chart: Chart,
    pub decls: Declarations,
    pub errors: Vec<syn::Error>,
}

struct Lower<'a> {
    chart_name: String,
    table: &'a mut SpanTable,
    errors: Vec<syn::Error>,
}

fn parse_ty(t: &TypeText, errors: &mut Vec<syn::Error>) -> Option<vertep_core::types::Type> {
    match parse_type(&t.text) {
        Ok(ty) => Some(ty),
        Err(e) => {
            errors.push(syn::Error::new(
                t.span,
                format!("bad type `{}`: {}", t.text, e.message),
            ));
            None
        }
    }
}

pub fn lower(ast: &ChartAst, table: &mut SpanTable) -> Lowered {
    let chart_name = ast
        .name
        .as_ref()
        .map_or_else(|| "chart".to_string(), |n| n.text.clone());
    let mut l = Lower {
        chart_name: chart_name.clone(),
        table,
        errors: Vec::new(),
    };
    let mut chart = Chart {
        name: Some(chart_name),
        datamodel: "rust".to_string(),
        datamodel_explicit: true,
        ..Chart::default()
    };
    if let Some(dm) = &ast.datamodel
        && dm.text != "rust"
    {
        l.errors.push(syn::Error::new(
            dm.span,
            format!(
                "statechart! charts use the `rust` datamodel, not `{}`; import charts on other \
                 datamodels with their SCXML or YAML readers",
                dm.text
            ),
        ));
    }
    if let Some(b) = &ast.binding {
        chart.binding_explicit = true;
        chart.binding = match b.text.as_str() {
            "early" => Binding::Early,
            "late" => Binding::Late,
            _ => {
                l.errors.push(syn::Error::new(
                    b.span,
                    "expected `early` or `late` binding",
                ));
                Binding::Early
            }
        };
    }
    chart.span = l.table.mark(ast.span);
    chart.initial = ast.initial.iter().map(|n| n.text.clone()).collect();
    if !ast.initial.is_empty() {
        l.table.targets.insert(chart.span.line, ast.initial.clone());
    }

    // Declarations.
    let mut decls = Declarations::default();
    for t in &ast.types {
        if let Some(ty) = parse_ty(&t.ty, &mut l.errors) {
            let span = l.table.mark(t.name.span);
            decls.types.push(TypeDecl {
                name: t.name.text.clone(),
                ty,
                span,
            });
        }
    }
    for e in &ast.events {
        let payload = match &e.payload {
            Some(p) => match parse_ty(p, &mut l.errors) {
                Some(t) => Some(t),
                None => continue,
            },
            None => None,
        };
        let span = l.table.mark(e.name.span);
        l.table
            .events
            .entry(e.name.text.clone())
            .or_insert(e.name.span);
        decls.events.push(EventDecl {
            name: e.name.text.clone(),
            payload,
            span,
        });
    }
    if !decls.is_empty() {
        set_declarations(&mut chart, Some(&decls));
    }

    chart.data = ast.context.iter().map(|d| l.data(d)).collect();
    chart.states = ast.states.iter().map(|s| l.state(s)).collect();
    Lowered {
        chart,
        decls,
        errors: l.errors,
    }
}

impl Lower<'_> {
    fn data(&mut self, d: &DataAst) -> Data {
        let mut data = Data {
            id: d.id.text.clone(),
            src: None,
            expr: d.init.as_ref().map(|i| i.text.clone()),
            content: None,
            extensions: Vec::new(),
            span: self.table.mark(d.id.span),
        };
        if let Some(t) = &d.ty
            && let Some(ty) = parse_ty(t, &mut self.errors)
        {
            set_data_type(&mut data, Some(&ty));
        }
        data
    }

    fn function(&mut self, name: &Named) {
        self.table
            .functions
            .entry(name.text.clone())
            .or_insert(name.span);
    }

    fn actions(&mut self, actions: &[ActionAst]) -> Vec<Exec> {
        actions.iter().map(|a| self.action(a)).collect()
    }

    fn action(&mut self, a: &ActionAst) -> Exec {
        match a {
            ActionAst::Call(name) => {
                self.function(name);
                let mut e = action_exec(&action_iri_for(&self.chart_name, &name.text));
                let span = self.table.mark(name.span);
                e.span = span;
                e
            }
            ActionAst::Raise(ev) => Exec {
                kind: ExecKind::Raise {
                    event: ev.text.clone(),
                },
                extensions: Vec::new(),
                span: self.table.mark(ev.span),
            },
            ActionAst::Send {
                event,
                target,
                delay,
                type_,
                id,
                span,
            } => Exec {
                kind: ExecKind::Send(Box::new(ir::Send {
                    event: Some(event.text.clone()),
                    target: target.clone(),
                    delay: delay.clone(),
                    type_: type_.clone(),
                    id: id.clone(),
                    ..ir::Send::default()
                })),
                extensions: Vec::new(),
                span: self.table.mark(*span),
            },
            ActionAst::Log(label) => Exec {
                kind: ExecKind::Log {
                    label: Some(label.text.clone()),
                    expr: None,
                },
                extensions: Vec::new(),
                span: self.table.mark(label.span),
            },
            ActionAst::Cancel(id) => Exec {
                kind: ExecKind::Cancel {
                    sendid: Some(id.text.clone()),
                    sendidexpr: None,
                },
                extensions: Vec::new(),
                span: self.table.mark(id.span),
            },
        }
    }

    fn guard(&mut self, g: &GuardAst) -> String {
        match g {
            GuardAst::Fn { name, negate } => {
                self.function(name);
                let iri = guard_iri_for(&self.chart_name, &name.text);
                if *negate { format!("!{iri}") } else { iri }
            }
            GuardAst::In { state, negate } => {
                let p = format!("In('{}')", state.text);
                if *negate { format!("!{p}") } else { p }
            }
        }
    }

    fn state(&mut self, s: &StateAst) -> State {
        let element = match s.kw {
            StateKw::State => StateElement::State,
            StateKw::Parallel => StateElement::Parallel,
            StateKw::Final => StateElement::Final,
            StateKw::History { deep } => StateElement::History(if deep {
                HistoryType::Deep
            } else {
                HistoryType::Shallow
            }),
        };
        self.table
            .state_decls
            .entry(s.id.text.clone())
            .or_default()
            .push(s.id.span);
        let mut st = State::new(s.id.text.clone(), element);
        st.span = self.table.mark(s.id.span);
        st.initial = s.initial.iter().map(|n| n.text.clone()).collect();
        if !s.initial.is_empty() {
            self.table.targets.insert(st.span.line, s.initial.clone());
        }
        st.data = s.data.iter().map(|d| self.data(d)).collect();
        st.onentry = s.entry.iter().map(|b| self.actions(b)).collect();
        st.onexit = s.exit.iter().map(|b| self.actions(b)).collect();
        for t in &s.transitions {
            let span = self.table.mark(t.span);
            self.table.targets.insert(span.line, t.targets.clone());
            let cond = t.guard.as_ref().map(|g| self.guard(g));
            st.transitions.push(Transition {
                events: t.events.iter().map(|e| e.text.clone()).collect(),
                cond,
                targets: t.targets.iter().map(|n| n.text.clone()).collect(),
                kind: if t.internal == Some(true) {
                    TransitionType::Internal
                } else {
                    TransitionType::External
                },
                kind_explicit: t.internal.is_some(),
                body: self.actions(&t.actions),
                extensions: Vec::new(),
                span,
            });
        }
        for inv in &s.invokes {
            let span = self.table.mark(inv.span);
            st.invokes.push(Invoke {
                type_: Some(inv.type_.clone()),
                src: inv.src.clone(),
                id: inv.id.clone(),
                autoforward: inv.autoforward,
                finalize: self.actions(&inv.finalize),
                span,
                ..Invoke::default()
            });
        }
        st.children = s.children.iter().map(|c| self.state(c)).collect();
        st
    }
}
