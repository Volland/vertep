//! Loading a chart into the CEL datamodel: every expression is parsed and checked against
//! the chart's type environment before the chart may run, and all errors are reported
//! together ([[openspec:cel-datamodel#Static Type Checking of CEL]]).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use vertep_core::ir::{Block, Chart, Content, Data, ExecKind, Param, State};

use crate::check::{CheckScope, check};
use crate::convert::{RESERVED_NAMES, coerce, location_type, parse_location};
use crate::datamodel::{CEL, CelDatamodel};
use crate::error::Limits;
use crate::funcs::P;
use crate::json;
use crate::policy::{self, Policy};
use crate::syntax::{Expr, parse};
use crate::types::{CelType, TypeEnv, assignable};

/// A problem found while loading a chart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    /// Where in the chart, such as ``state `cart` › transition 1 › cond``.
    pub at: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// Every problem found while loading a chart; a chart with any is not instantiated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadErrors(pub Vec<LoadError>);

impl fmt::Display for LoadErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl core::error::Error for LoadError {}

impl core::error::Error for LoadErrors {}

/// How to load a chart.
#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    /// The chart's CEL type environment (untyped by default).
    pub env: TypeEnv,
    /// Evaluation limits.
    pub limits: Limits,
    /// The loading policy.
    pub policy: Policy,
}

/// What an expression must evaluate to where it occurs.
#[derive(Clone, Debug)]
enum Expect {
    Any,
    Bool,
    String,
    Iterable,
    Assign(CelType, String),
}

/// Parse and check every expression of `chart`, apply the policy, and return a CEL
/// datamodel primed with the parsed expressions, or every error found.
// @tg: implements:: [[openspec:cel-datamodel#Static Type Checking of CEL]]
pub fn load(chart: &Chart, options: &LoadOptions) -> Result<CelDatamodel, LoadErrors> {
    let mut l = Loader {
        env: &options.env,
        limits: options.limits,
        errors: Vec::new(),
        programs: BTreeMap::new(),
        loci: BTreeMap::new(),
        scripts: options.policy == Policy::Standard,
    };
    if let Err(mut v) = policy::check(chart, options.policy) {
        l.errors.append(&mut v);
    }
    if chart.datamodel != CEL && options.policy == Policy::Standard {
        l.error(
            "chart",
            format!(
                "the chart declares datamodel `{}`; the CEL datamodel loads only charts with datamodel `cel`",
                chart.datamodel
            ),
        );
    }
    l.data(&chart.data, "chart");
    if chart.script.is_some() && options.policy == Policy::Standard {
        l.error("chart › <script>", String::from(SCRIPT_MSG));
    }
    for s in &chart.states {
        l.state(s);
    }
    if !l.errors.is_empty() {
        return Err(LoadErrors(l.errors));
    }
    let mut dm = CelDatamodel::new(options.env.clone(), options.limits);
    dm.programs = l.programs;
    dm.loci = l.loci;
    Ok(dm)
}

const SCRIPT_MSG: &str =
    "<script> is not supported by the CEL datamodel: CEL is expression-only; use <assign>";

struct Loader<'a> {
    env: &'a TypeEnv,
    limits: Limits,
    errors: Vec<LoadError>,
    programs: BTreeMap<String, Arc<Expr>>,
    loci: BTreeMap<String, Vec<String>>,
    /// Report `<script>` here (under the CEL-only policy the policy reports it).
    scripts: bool,
}

impl Loader<'_> {
    fn error(&mut self, at: &str, message: String) {
        self.errors.push(LoadError {
            at: String::from(at),
            message,
        });
    }

    /// Parse and check `src`; returns its type.
    fn expr(
        &mut self,
        src: &str,
        at: &str,
        event_data: &CelType,
        expect: Expect,
    ) -> Option<CelType> {
        self.loci
            .entry(String::from(src))
            .or_default()
            .push(String::from(at));
        if src.len() > self.limits.max_expr_len {
            self.error(
                at,
                format!("expression longer than {} bytes", self.limits.max_expr_len),
            );
            return None;
        }
        let e = match parse(src, self.limits.max_depth) {
            Ok(e) => e,
            Err(e) => {
                self.error(
                    at,
                    format!("syntax error at offset {} in `{src}`: {}", e.pos, e.message),
                );
                return None;
            }
        };
        let scope = CheckScope {
            env: self.env,
            event_data: event_data.clone(),
        };
        let t = match check(&e, &scope) {
            Ok(t) => t,
            Err(errs) => {
                for c in errs {
                    self.error(at, format!("{} in `{src}`", c.message));
                }
                return None;
            }
        };
        self.programs.insert(String::from(src), Arc::new(e));
        match expect {
            Expect::Any => {}
            Expect::Bool => {
                if !P::Bool.accepts(&t) {
                    self.error(
                        at,
                        format!("a guard must have type bool, but `{src}` has type {t}"),
                    );
                }
            }
            Expect::String => {
                if !P::String.accepts(&t) {
                    self.error(at, format!("`{src}` must have type string, found {t}"));
                }
            }
            Expect::Iterable => {
                if !(P::List.accepts(&t) || P::Map.accepts(&t)) {
                    self.error(
                        at,
                        format!("foreach needs a list or a map, but `{src}` has type {t}"),
                    );
                }
            }
            Expect::Assign(to, loc) => {
                if !assignable(&to, &t) {
                    self.error(at, format!("cannot assign {t} to `{loc}` of type {to}"));
                }
            }
        }
        Some(t)
    }

    /// Check a location; returns its declared type.
    fn location(&mut self, src: &str, at: &str) -> Option<CelType> {
        let loc = match parse_location(src, self.limits.max_depth) {
            Ok(l) => l,
            Err(e) => {
                self.error(at, e.message);
                return None;
            }
        };
        for s in &loc.path {
            if let crate::convert::Seg::Index(e) = s {
                let scope = CheckScope {
                    env: self.env,
                    event_data: CelType::Dyn,
                };
                if let Err(errs) = check(e, &scope) {
                    for c in errs {
                        self.error(at, format!("{} in `{src}`", c.message));
                    }
                }
            }
        }
        match location_type(self.env, &loc) {
            Ok(t) => Some(t),
            Err(m) => {
                self.error(at, format!("`{src}`: {m}"));
                None
            }
        }
    }

    fn assign(
        &mut self,
        location: &str,
        expr: Option<&str>,
        content: Option<&str>,
        at: &str,
        ed: &CelType,
    ) {
        let Some(ty) = self.location(location, &format!("{at} › location")) else {
            if let Some(x) = expr {
                self.expr(x, &format!("{at} › expr"), ed, Expect::Any);
            }
            return;
        };
        let shown = if location.starts_with("ctx") {
            String::from(location)
        } else {
            format!("ctx.{location}")
        };
        if let Some(x) = expr {
            self.expr(x, &format!("{at} › expr"), ed, Expect::Assign(ty, shown));
        } else if let Some(c) = content
            && let Some(v) = json::parse(c, self.limits.max_depth)
            && let Err(e) = coerce(v, &ty, &shown)
        {
            self.error(&format!("{at} › content"), e.message);
        }
    }

    fn data(&mut self, items: &[Data], at: &str) {
        for d in items {
            let here = format!("{at} › <data id=\"{}\">", d.id);
            if RESERVED_NAMES.contains(&d.id.as_str()) {
                self.error(
                    &here,
                    format!("`{}` is reserved and cannot be a data item", d.id),
                );
                continue;
            }
            if let Err(m) = self.env.context_field(&d.id) {
                self.error(&here, m);
                continue;
            }
            let loc = format!("ctx[{:?}]", d.id);
            let target = if is_ident(&d.id) { d.id.clone() } else { loc };
            self.assign(
                &target,
                d.expr.as_deref(),
                d.content.as_deref(),
                &here,
                &CelType::Dyn,
            );
        }
    }

    fn state(&mut self, s: &State) {
        let at = format!("state `{}`", s.id);
        self.data(&s.data, &at);
        if let Some(init) = &s.initial_element {
            self.block(
                &init.transition.body,
                &format!("{at} › <initial>"),
                &CelType::Dyn,
            );
        }
        for (i, t) in s.transitions.iter().enumerate() {
            let here = format!("{at} › transition {}", i + 1);
            let ed = self.env.event_data_for(&t.events);
            if let Some(c) = &t.cond {
                self.expr(c, &format!("{here} › cond"), &ed, Expect::Bool);
            }
            self.block(&t.body, &here, &ed);
        }
        for (i, b) in s.onentry.iter().enumerate() {
            self.block(b, &format!("{at} › <onentry> {}", i + 1), &CelType::Dyn);
        }
        for (i, b) in s.onexit.iter().enumerate() {
            self.block(b, &format!("{at} › <onexit> {}", i + 1), &CelType::Dyn);
        }
        for (i, inv) in s.invokes.iter().enumerate() {
            let here = format!("{at} › <invoke> {}", i + 1);
            if let Some(x) = &inv.typeexpr {
                self.expr(
                    x,
                    &format!("{here} › typeexpr"),
                    &CelType::Dyn,
                    Expect::String,
                );
            }
            if let Some(x) = &inv.srcexpr {
                self.expr(
                    x,
                    &format!("{here} › srcexpr"),
                    &CelType::Dyn,
                    Expect::String,
                );
            }
            if let Some(l) = &inv.idlocation {
                self.idlocation(l, &here);
            }
            self.payload(&inv.namelist, &inv.params, inv.content.as_ref(), &here);
            self.block(
                &inv.finalize,
                &format!("{here} › <finalize>"),
                &CelType::Dyn,
            );
        }
        if let Some(dd) = &s.donedata {
            let here = format!("{at} › <donedata>");
            self.payload(&[], &dd.params, dd.content.as_ref(), &here);
        }
        for c in &s.children {
            self.state(c);
        }
    }

    fn idlocation(&mut self, l: &str, at: &str) {
        let here = format!("{at} › idlocation");
        if let Some(t) = self.location(l, &here)
            && !assignable(&t, &CelType::String)
        {
            self.error(&here, format!("cannot assign string to `{l}` of type {t}"));
        }
    }

    fn payload(
        &mut self,
        namelist: &[String],
        params: &[Param],
        content: Option<&Content>,
        at: &str,
    ) {
        for n in namelist {
            self.location(n, &format!("{at} › namelist `{n}`"));
        }
        for p in params {
            let here = format!("{at} › <param name=\"{}\">", p.name);
            if let Some(x) = &p.expr {
                self.expr(x, &here, &CelType::Dyn, Expect::Any);
            } else if let Some(l) = &p.location {
                self.location(l, &here);
            }
        }
        if let Some(Content { expr: Some(x), .. }) = content {
            self.expr(x, &format!("{at} › <content>"), &CelType::Dyn, Expect::Any);
        }
    }

    fn block(&mut self, b: &Block, at: &str, ed: &CelType) {
        for (i, e) in b.iter().enumerate() {
            let n = i + 1;
            match &e.kind {
                ExecKind::Raise { .. } | ExecKind::Extension(_) => {}
                ExecKind::Send(s) => {
                    let here = format!("{at} › <send> {n}");
                    for (x, attr) in [
                        (&s.eventexpr, "eventexpr"),
                        (&s.targetexpr, "targetexpr"),
                        (&s.typeexpr, "typeexpr"),
                        (&s.delayexpr, "delayexpr"),
                    ] {
                        if let Some(x) = x {
                            self.expr(x, &format!("{here} › {attr}"), ed, Expect::String);
                        }
                    }
                    if let Some(l) = &s.idlocation {
                        self.idlocation(l, &here);
                    }
                    self.payload(&s.namelist, &s.params, s.content.as_ref(), &here);
                }
                ExecKind::Cancel { sendidexpr, .. } => {
                    if let Some(x) = sendidexpr {
                        self.expr(
                            x,
                            &format!("{at} › <cancel> {n} › sendidexpr"),
                            ed,
                            Expect::String,
                        );
                    }
                }
                ExecKind::Log { expr, .. } => {
                    if let Some(x) = expr {
                        self.expr(x, &format!("{at} › <log> {n}"), ed, Expect::Any);
                    }
                }
                ExecKind::Assign {
                    location,
                    expr,
                    content,
                } => {
                    self.assign(
                        location,
                        expr.as_deref(),
                        content.as_deref(),
                        &format!("{at} › <assign> {n}"),
                        ed,
                    );
                }
                ExecKind::Script(_) => {
                    if self.scripts {
                        self.error(&format!("{at} › <script> {n}"), String::from(SCRIPT_MSG));
                    }
                }
                ExecKind::If { branches } => {
                    for (j, br) in branches.iter().enumerate() {
                        let here = format!("{at} › <if> {n} › branch {}", j + 1);
                        if let Some(c) = &br.cond {
                            self.expr(c, &format!("{here} › cond"), ed, Expect::Bool);
                        }
                        self.block(&br.body, &here, ed);
                    }
                }
                ExecKind::Foreach {
                    array,
                    item,
                    index,
                    body,
                } => {
                    let here = format!("{at} › <foreach> {n}");
                    let t = self.expr(array, &format!("{here} › array"), ed, Expect::Iterable);
                    let el = match t.as_ref().map(CelType::strip_optional) {
                        Some(CelType::List(el)) => (**el).clone(),
                        Some(CelType::Map(k, _)) => (**k).clone(),
                        _ => CelType::Dyn,
                    };
                    self.foreach_var(item, &el, &format!("{here} › item"));
                    if let Some(ix) = index {
                        self.foreach_var(ix, &CelType::Int, &format!("{here} › index"));
                    }
                    self.block(body, &here, ed);
                }
            }
        }
    }

    fn foreach_var(&mut self, name: &str, ty: &CelType, at: &str) {
        if let Some(t) = self.location(name, at)
            && !assignable(&t, ty)
        {
            self.error(at, format!("cannot assign {ty} to `{name}` of type {t}"));
        }
    }
}

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod error_tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn load_errors_box_with_question_mark() {
        let e = LoadError {
            at: "state `s` › transition 1 › cond".into(),
            message: "a guard must have type bool".into(),
        };
        let _: &dyn core::error::Error = &e;
        fn run(e: LoadError) -> Result<(), alloc::boxed::Box<dyn core::error::Error>> {
            Err(LoadErrors(vec![e.clone(), e]))?
        }
        let msg = run(e).unwrap_err().to_string();
        assert_eq!(msg.lines().count(), 2);
        assert!(msg.starts_with("state `s` › transition 1 › cond: a guard"));
    }
}
