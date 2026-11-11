//! The CEL-only loading policy for charts from untrusted authors
//! ([[openspec:cel-datamodel#Safe Datamodel for Generated Charts]]).
//!
//! Under [`Policy::CelOnly`] a chart must declare `datamodel="cel"`, must contain no
//! `<script>` (at the top level or in any executable content), and must not reference any
//! registered Rust guard or action by IRI (`vt:guard/…`, `vt:action/…` or their full
//! `https://w3id.org/vertep/vt#` forms) in any expression or extension attribute or element.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use vertep_core::ir::{
    Block, Chart, Content, ExecKind, Extension, ExtensionValue, Param, State, VT_NS,
};

use crate::datamodel::CEL;
use crate::load::LoadError;

/// Which charts a load accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    /// Any chart with `datamodel="cel"`; `<script>` is still rejected because CEL has none.
    #[default]
    Standard,
    /// Charts written by untrusted authors: CEL only, no scripts, no Rust function IRIs.
    CelOnly,
}

/// Check `chart` against `policy`, returning every violation.
// @tg: implements:: [[openspec:cel-datamodel#Safe Datamodel for Generated Charts]]
pub fn check(chart: &Chart, policy: Policy) -> Result<(), Vec<LoadError>> {
    if policy == Policy::Standard {
        return Ok(());
    }
    let mut w = Walker { errors: Vec::new() };
    if chart.datamodel != CEL {
        w.error(
            "chart",
            format!(
                "the CEL-only policy permits only datamodel `cel`, but the chart declares `{}`",
                chart.datamodel
            ),
        );
    }
    if chart.script.is_some() {
        w.error("chart › <script>", String::from(SCRIPT));
    }
    w.extensions(&chart.extensions, "chart");
    for d in &chart.data {
        w.data(d, "chart");
    }
    for s in &chart.states {
        w.state(s);
    }
    if w.errors.is_empty() {
        Ok(())
    } else {
        Err(w.errors)
    }
}

const SCRIPT: &str = "the CEL-only policy forbids the <script> element";

const FUNCTION_PREFIXES: &[&str] = &["vt:guard/", "vt:action/"];

/// Every Rust guard or action IRI mentioned in `text`.
pub fn function_iris(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let full_guard = format!("{VT_NS}guard/");
    let full_action = format!("{VT_NS}action/");
    let prefixes = [
        FUNCTION_PREFIXES[0],
        FUNCTION_PREFIXES[1],
        full_guard.as_str(),
        full_action.as_str(),
    ];
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        if let Some(p) = prefixes.iter().find(|p| rest.starts_with(**p)) {
            let end = rest[p.len()..]
                .find(|c: char| c.is_whitespace() || "\"'<>(),;`".contains(c))
                .map_or(rest.len(), |e| e + p.len());
            out.push(String::from(&rest[..end]));
            i += end.max(1);
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

struct Walker {
    errors: Vec<LoadError>,
}

impl Walker {
    fn error(&mut self, at: &str, message: String) {
        self.errors.push(LoadError {
            at: String::from(at),
            message,
        });
    }

    fn text(&mut self, text: &str, at: &str) {
        for iri in function_iris(text) {
            self.error(
                at,
                format!(
                    "the CEL-only policy forbids references to registered Rust functions: `{iri}`"
                ),
            );
        }
    }

    fn extensions(&mut self, exts: &[Extension], at: &str) {
        for x in exts {
            let (ExtensionValue::Attribute(v) | ExtensionValue::Element(v)) = &x.value;
            self.text(v, &format!("{at} › {}:{}", x.ns, x.local));
        }
    }

    fn data(&mut self, d: &vertep_core::ir::Data, at: &str) {
        let here = format!("{at} › <data id=\"{}\">", d.id);
        if let Some(x) = &d.expr {
            self.text(x, &here);
        }
        self.extensions(&d.extensions, &here);
    }

    fn opt(&mut self, x: &Option<String>, at: &str) {
        if let Some(x) = x {
            self.text(x, at);
        }
    }

    fn payload(&mut self, params: &[Param], content: Option<&Content>, at: &str) {
        for p in params {
            self.opt(&p.expr, at);
            self.opt(&p.location, at);
        }
        if let Some(c) = content {
            self.opt(&c.expr, at);
        }
    }

    fn state(&mut self, s: &State) {
        let at = format!("state `{}`", s.id);
        self.extensions(&s.extensions, &at);
        for d in &s.data {
            self.data(d, &at);
        }
        if let Some(init) = &s.initial_element {
            self.block(&init.transition.body, &format!("{at} › <initial>"));
        }
        for (i, t) in s.transitions.iter().enumerate() {
            let here = format!("{at} › transition {}", i + 1);
            self.opt(&t.cond, &format!("{here} › cond"));
            self.extensions(&t.extensions, &here);
            self.block(&t.body, &here);
        }
        for b in s.onentry.iter().chain(s.onexit.iter()) {
            self.block(b, &at);
        }
        for inv in &s.invokes {
            let here = format!("{at} › <invoke>");
            self.opt(&inv.typeexpr, &here);
            self.opt(&inv.srcexpr, &here);
            self.payload(&inv.params, inv.content.as_ref(), &here);
            self.extensions(&inv.extensions, &here);
            self.block(&inv.finalize, &here);
        }
        if let Some(dd) = &s.donedata {
            self.payload(
                &dd.params,
                dd.content.as_ref(),
                &format!("{at} › <donedata>"),
            );
        }
        for c in &s.children {
            self.state(c);
        }
    }

    fn block(&mut self, b: &Block, at: &str) {
        for e in b {
            self.extensions(&e.extensions, at);
            match &e.kind {
                ExecKind::Script(_) => {
                    self.error(&format!("{at} › <script>"), String::from(SCRIPT))
                }
                ExecKind::Send(s) => {
                    let here = format!("{at} › <send>");
                    for x in [&s.eventexpr, &s.targetexpr, &s.typeexpr, &s.delayexpr] {
                        self.opt(x, &here);
                    }
                    self.payload(&s.params, s.content.as_ref(), &here);
                }
                ExecKind::Cancel { sendidexpr, .. } => self.opt(sendidexpr, at),
                ExecKind::Log { expr, .. } => self.opt(expr, &format!("{at} › <log>")),
                ExecKind::Assign { location, expr, .. } => {
                    let here = format!("{at} › <assign>");
                    self.text(location, &here);
                    self.opt(expr, &here);
                }
                ExecKind::If { branches } => {
                    for br in branches {
                        self.opt(&br.cond, &format!("{at} › <if>"));
                        self.block(&br.body, at);
                    }
                }
                ExecKind::Foreach { array, body, .. } => {
                    self.text(array, &format!("{at} › <foreach>"));
                    self.block(body, at);
                }
                ExecKind::Extension(x) => {
                    let (ExtensionValue::Attribute(v) | ExtensionValue::Element(v)) = &x.value;
                    self.text(v, &format!("{at} › {}:{}", x.ns, x.local));
                }
                ExecKind::Raise { .. } => {}
            }
        }
    }
}
