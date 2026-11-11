//! Rewriting every span of a chart.

use vertep_core::ir::{Block, Chart, Exec, ExecKind, Span, State, Transition};

fn transition(t: &mut Transition, f: &dyn Fn(Span) -> Span) {
    t.span = f(t.span);
    block(&mut t.body, f);
    t.extensions.iter_mut().for_each(|e| e.span = f(e.span));
}

fn block(b: &mut Block, f: &dyn Fn(Span) -> Span) {
    b.iter_mut().for_each(|e| exec(e, f));
}

fn exec(e: &mut Exec, f: &dyn Fn(Span) -> Span) {
    e.span = f(e.span);
    e.extensions.iter_mut().for_each(|x| x.span = f(x.span));
    match &mut e.kind {
        ExecKind::Send(s) => {
            s.params.iter_mut().for_each(|p| p.span = f(p.span));
            if let Some(c) = &mut s.content {
                c.span = f(c.span);
            }
        }
        ExecKind::Script(s) => s.span = f(s.span),
        ExecKind::If { branches } => {
            for br in branches {
                br.span = f(br.span);
                block(&mut br.body, f);
            }
        }
        ExecKind::Foreach { body, .. } => block(body, f),
        ExecKind::Extension(x) => x.span = f(x.span),
        _ => {}
    }
}

fn state(s: &mut State, f: &dyn Fn(Span) -> Span) {
    s.span = f(s.span);
    s.extensions.iter_mut().for_each(|e| e.span = f(e.span));
    if let Some(i) = &mut s.initial_element {
        i.span = f(i.span);
        transition(&mut i.transition, f);
    }
    s.transitions.iter_mut().for_each(|t| transition(t, f));
    s.onentry.iter_mut().for_each(|b| block(b, f));
    s.onexit.iter_mut().for_each(|b| block(b, f));
    s.data.iter_mut().for_each(|d| {
        d.span = f(d.span);
        d.extensions.iter_mut().for_each(|e| e.span = f(e.span));
    });
    for inv in &mut s.invokes {
        inv.span = f(inv.span);
        inv.params.iter_mut().for_each(|p| p.span = f(p.span));
        if let Some(c) = &mut inv.content {
            c.span = f(c.span);
        }
        block(&mut inv.finalize, f);
    }
    if let Some(d) = &mut s.donedata {
        d.span = f(d.span);
        d.params.iter_mut().for_each(|p| p.span = f(p.span));
    }
    s.children.iter_mut().for_each(|c| state(c, f));
}

/// Apply `f` to every span in the chart.
pub fn map(chart: &mut Chart, f: &dyn Fn(Span) -> Span) {
    chart.span = f(chart.span);
    chart.extensions.iter_mut().for_each(|e| e.span = f(e.span));
    chart.data.iter_mut().for_each(|d| {
        d.span = f(d.span);
        d.extensions.iter_mut().for_each(|e| e.span = f(e.span));
    });
    if let Some(s) = &mut chart.script {
        s.span = f(s.span);
    }
    chart.states.iter_mut().for_each(|s| state(s, f));
}
