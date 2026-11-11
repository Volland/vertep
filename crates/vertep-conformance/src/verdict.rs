//! The W3C pass/fail protocol: a test passes when its top-level session enters the final state
//! `pass` and fails when it enters `fail`, any other top-level final state, runs out of its
//! step or virtual-time budget, or hits an interpreter error.

use std::fmt;

use serde::Serialize;
use vertep_core::compile::{CompiledChart, ROOT};
use vertep_core::ir::StateKind;
use vertep_core::trace::{MacrostepTrace, TraceInput};

use crate::host::Stop;

/// Why a test failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "kebab-case")]
pub enum FailReason {
    /// The session entered the final state `fail`.
    FailState,
    /// The session entered another top-level final state.
    OtherFinal(String),
    /// The step or virtual-time budget ran out before a verdict.
    BudgetExceeded(String),
    /// The session waits for an event that will never come.
    Stalled,
    /// The document could not be read, compiled or started, a macrostep did not stabilize, or
    /// the interpreter panicked.
    InterpreterError(String),
    /// A second engine (the Lean oracle) could not replay the Rust run or produced a different
    /// trace.
    Divergence(String),
}

impl fmt::Display for FailReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FailReason::FailState => f.write_str("entered final state `fail`"),
            FailReason::OtherFinal(s) => write!(f, "entered top-level final state `{s}`"),
            FailReason::BudgetExceeded(m) => write!(f, "budget exceeded: {m}"),
            FailReason::Stalled => f.write_str("stalled: no event pending and none scheduled"),
            FailReason::InterpreterError(m) => write!(f, "interpreter error: {m}"),
            FailReason::Divergence(m) => write!(f, "differs from the Rust core: {m}"),
        }
    }
}

/// The verdict of one run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", content = "reason", rename_all = "lowercase")]
pub enum Verdict {
    /// The session entered `pass`.
    Pass,
    /// The test failed.
    Fail(FailReason),
}

impl Verdict {
    /// Whether this is a pass.
    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass)
    }

    /// The failure reason, if any.
    pub fn reason(&self) -> Option<&FailReason> {
        match self {
            Verdict::Pass => None,
            Verdict::Fail(r) => Some(r),
        }
    }
}

/// The top-level final state a terminating macrostep entered, if it ended in one.
///
/// A top-level final state is a child of the root, so it is entered either as an explicit
/// target of the transition that ended the session or as a target of the root's initial
/// transition; the trace names the former and the chart the latter.
pub fn entered_final(chart: &CompiledChart, trace: &MacrostepTrace) -> Option<String> {
    if !trace.terminated {
        return None;
    }
    let top_final = |s: usize| {
        let st = chart.state(s);
        (st.parent == Some(ROOT) && st.kind == StateKind::Final).then(|| st.id.clone())
    };
    for ms in trace.microsteps.iter().rev() {
        for r in &ms.transitions {
            let Some(src) = chart.index_of(&r.source) else {
                continue;
            };
            let found = chart.state(src).transitions.iter().find_map(|&t| {
                let tr = chart.transition(t);
                (tr.index == Some(r.index))
                    .then(|| tr.targets.iter().find_map(|&s| top_final(s)))
                    .flatten()
            });
            if found.is_some() {
                return found;
            }
        }
    }
    if matches!(trace.input, TraceInput::Start { .. })
        && let Some(it) = chart.state(ROOT).initial
    {
        return chart
            .transition(it)
            .targets
            .iter()
            .find_map(|&s| top_final(s));
    }
    None
}

/// Judge a finished host run of a test document from its root records
/// ([[openspec:scxml-conformance#Pass Fail Protocol]]).
// @tg: implements:: [[openspec:scxml-conformance#Pass Fail Protocol]]
pub fn judge(chart: &CompiledChart, last: Option<&MacrostepTrace>, stop: &Stop) -> Verdict {
    match stop {
        Stop::Finished => match last.and_then(|t| entered_final(chart, t)) {
            Some(s) if s == "pass" => Verdict::Pass,
            Some(s) if s == "fail" => Verdict::Fail(FailReason::FailState),
            Some(s) => Verdict::Fail(FailReason::OtherFinal(s)),
            None => Verdict::Fail(FailReason::InterpreterError(
                "the session ended without entering a top-level final state".to_string(),
            )),
        },
        Stop::StepBudget(_) | Stop::TimeBudget(_) => {
            Verdict::Fail(FailReason::BudgetExceeded(stop.to_string()))
        }
        Stop::Stalled => Verdict::Fail(FailReason::Stalled),
        Stop::MicrostepBudget { .. } | Stop::StartFailed(_) => {
            Verdict::Fail(FailReason::InterpreterError(stop.to_string()))
        }
    }
}
