//! Document order over the chart IR.
//!
//! The IR keeps every ordered collection (children, transitions, targets, executable content
//! blocks) in source order, so document order is a pre-order walk of the tree. The walk visits a
//! state, then its `<initial>` transition, its `onentry` and `onexit` blocks, its transitions
//! (each followed by its content), its invokes' `finalize` blocks, and finally its child states.
//! Positions are the indices of the returned sequence.
//!
//! States alone keep the same relative order in the compiled chart, whose dense state indices are
//! document positions restricted to states.

use alloc::vec::Vec;

use crate::ir::{Block, Chart, Exec, ExecKind, State, Transition};

/// A node visited by the document-order walk.
#[derive(Clone, Copy, Debug)]
pub enum DocNode<'a> {
    /// A state of any kind.
    State(&'a State),
    /// A transition declared on a state, with its index in the state's transition list.
    Transition {
        /// Identifier of the source state.
        source: &'a str,
        /// Index in `State::transitions`.
        index: usize,
        /// The transition.
        transition: &'a Transition,
    },
    /// The transition of a state's `<initial>` element.
    InitialTransition {
        /// Identifier of the owning compound state.
        state: &'a str,
        /// The transition.
        transition: &'a Transition,
    },
    /// One executable content element.
    Exec(&'a Exec),
}

/// Every state, transition and executable content element of `chart` in document order; the
/// position of a node is its index in the result.
// @tg: implements:: [[openspec:chart-model#Document Order]]
pub fn document_order(chart: &Chart) -> Vec<DocNode<'_>> {
    let mut out = Vec::new();
    for s in &chart.states {
        walk_state(s, &mut out);
    }
    out
}

/// The document position of every state, in the same pre-order as [`document_order`]. The
/// result lists state identifiers; a state's position is its index.
pub fn state_order(chart: &Chart) -> Vec<&str> {
    let mut out = Vec::new();
    fn go<'a>(s: &'a State, out: &mut Vec<&'a str>) {
        out.push(s.id.as_str());
        for c in &s.children {
            go(c, out);
        }
    }
    for s in &chart.states {
        go(s, &mut out);
    }
    out
}

fn walk_state<'a>(s: &'a State, out: &mut Vec<DocNode<'a>>) {
    out.push(DocNode::State(s));
    if let Some(init) = &s.initial_element {
        out.push(DocNode::InitialTransition {
            state: s.id.as_str(),
            transition: &init.transition,
        });
        walk_block(&init.transition.body, out);
    }
    for b in &s.onentry {
        walk_block(b, out);
    }
    for b in &s.onexit {
        walk_block(b, out);
    }
    for (index, t) in s.transitions.iter().enumerate() {
        out.push(DocNode::Transition {
            source: s.id.as_str(),
            index,
            transition: t,
        });
        walk_block(&t.body, out);
    }
    for inv in &s.invokes {
        walk_block(&inv.finalize, out);
    }
    for c in &s.children {
        walk_state(c, out);
    }
}

fn walk_block<'a>(b: &'a Block, out: &mut Vec<DocNode<'a>>) {
    for e in b {
        out.push(DocNode::Exec(e));
        match &e.kind {
            ExecKind::If { branches } => {
                for br in branches {
                    walk_block(&br.body, out);
                }
            }
            ExecKind::Foreach { body, .. } => walk_block(body, out),
            _ => {}
        }
    }
}

/// Structural chart equality: every node, attribute, expression text, extension node and
/// document order is compared, source locations are not (a [`crate::ir::Span`] equals every
/// other span). Two charts are equal exactly when every surface serializes them identically.
// @tg: implements:: [[openspec:chart-model#Chart Equality]]
pub fn same_chart(a: &Chart, b: &Chart) -> bool {
    a == b
}
