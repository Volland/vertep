//! How guards and actions of the `rust` datamodel appear in the chart IR.
//!
//! The IR is shared by every surface and keeps expressions as text, so the `rust` datamodel
//! gives its expressions a small, fixed grammar:
//!
//! - A **guard** is a transition `cond` (or `<if>` condition): an IRI naming a registered
//!   guard, optionally negated with `!`, or `In('state')`, `true` or `false`.
//!   `statechart!` writes `on pay if positive => payment;` as `cond="vt:guard/order/positive"`.
//! - An **action** is one `<foreach>` whose `array` is the action's IRI and whose body is one
//!   `<send>` reading the loop variable `_vt_emit`:
//!
//! ```xml
//! <foreach array="vt:action/order/record_total" item="_vt_emit">
//!   <send eventexpr="_vt_emit.event" targetexpr="_vt_emit.target"
//!         typeexpr="_vt_emit.type" delayexpr="_vt_emit.delay">
//!     <content expr="_vt_emit.data"/>
//!   </send>
//! </foreach>
//! ```
//!
//!   Evaluating the `array` runs the action, which may change the context, and yields the
//!   events it emitted; the body turns each into a `<send>` — to `#_internal` for a raise, so it
//!   is processed in the same macrostep, and otherwise an effect returned by the step. This
//!   keeps every effect of an action visible in the IR and in the step output without any
//!   change to the interpreter.
//! - **Context** is the chart's `<data>` items; initializers are JSON literals (`0`, `"x"`,
//!   `true`, `null`, `[]`, `{}`).
//!
//! The default IRIs are `vt:guard/<chart>/<name>` and `vt:action/<chart>/<name>`, where `vt:`
//! abbreviates the vt namespace.

use vertep_core::ir::{Content, Exec, ExecKind, Send};
use vertep_core::types::prim::check_iri;

/// The prefix of default guard IRIs.
pub const GUARD_PREFIX: &str = "vt:guard/";
/// The prefix of default action IRIs.
pub const ACTION_PREFIX: &str = "vt:action/";
/// The loop variable a lowered action's `<send>` reads.
pub const EMIT_VAR: &str = "_vt_emit";

/// The default IRI of guard `name` in chart `chart`.
pub fn guard_iri_for(chart: &str, name: &str) -> String {
    format!("{GUARD_PREFIX}{chart}/{name}")
}

/// The default IRI of action `name` in chart `chart`.
pub fn action_iri_for(chart: &str, name: &str) -> String {
    format!("{ACTION_PREFIX}{chart}/{name}")
}

/// Whether `text` is an IRI with a scheme (and so names a function rather than a literal or a
/// variable path).
pub fn is_iri(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && !t.starts_with(['"', '\'']) && check_iri(t).is_ok()
}

/// The guard IRI a condition names, after any `!`, if it names one.
pub fn guard_iri(cond: &str) -> Option<&str> {
    let c = cond.trim().trim_start_matches(['!', ' ']);
    is_iri(c).then_some(c.trim())
}

/// The executable content an action call lowers to.
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
            span: Default::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_function_iris() {
        assert!(is_iri("vt:guard/order/positive"));
        assert!(is_iri("https://example.org/g#x"));
        assert!(!is_iri("_vt_emit.data"));
        assert!(!is_iri("In('a')"));
        assert!(!is_iri("\"a:b\""));
        assert_eq!(guard_iri("!vt:guard/a/b"), Some("vt:guard/a/b"));
        assert_eq!(guard_iri("true"), None);
    }
}
