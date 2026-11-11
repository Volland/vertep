//! How the `rust` datamodel's guards and actions appear in the IR.
//!
//! This mirrors `vertep_dm_rust::lowering`, which the runtime interprets; a proc-macro crate
//! cannot depend on the runtime crate that re-exports it, so the few conventions are repeated
//! here and an equality test in `vertep-dm-rust` keeps the two in step.

use vertep_core::ir::{Content, Exec, ExecKind, Send, Span};
use vertep_core::types::prim::check_iri;

pub const GUARD_PREFIX: &str = "vt:guard/";
pub const ACTION_PREFIX: &str = "vt:action/";
pub const EMIT_VAR: &str = "_vt_emit";

pub fn guard_iri_for(chart: &str, name: &str) -> String {
    format!("{GUARD_PREFIX}{chart}/{name}")
}

pub fn action_iri_for(chart: &str, name: &str) -> String {
    format!("{ACTION_PREFIX}{chart}/{name}")
}

pub fn is_iri(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && !t.starts_with(['"', '\'']) && check_iri(t).is_ok()
}

/// The guard IRI a condition names, after any `!`.
pub fn guard_iri(cond: &str) -> Option<&str> {
    let c = cond.trim().trim_start_matches(['!', ' ']);
    is_iri(c).then_some(c.trim())
}

/// The function name an IRI binds to: its last path or fragment segment.
pub fn function_name(iri: &str) -> &str {
    iri.rsplit(['/', '#', ':']).next().unwrap_or(iri)
}

pub fn action_exec(iri: &str) -> Exec {
    let field = |f: &str| Some(format!("{EMIT_VAR}.{f}"));
    let send = Send {
        eventexpr: field("event"),
        targetexpr: field("target"),
        typeexpr: field("type"),
        delayexpr: field("delay"),
        content: Some(Content {
            expr: field("data"),
            body: None,
            span: Span::NONE,
        }),
        ..Send::default()
    };
    Exec::new(ExecKind::Foreach {
        array: iri.to_string(),
        item: EMIT_VAR.to_string(),
        index: None,
        body: vec![Exec::new(ExecKind::Send(Box::new(send)))],
    })
}
