//! Plan charts: chart fragments written by an LLM, parsed, validated, restricted to the CEL or
//! null datamodel, content-addressed and run only as bounded, sandboxed invoked children.
//!
//! Submission ([`submit`]) parses a plan from YAML or Turtle into the IR and validates it with
//! the same IR well-formedness checks and SHACL chart shapes as any chart, plus the plan
//! restrictions ([[decisions#Plan charts may use the null datamodel]]): the datamodel is `cel`
//! or `null`; there is no `<script>`; every Rust guard or action IRI and every `vt:call` names
//! a granted function; and CEL expressions type-check under the CEL-only policy. An accepted
//! plan is named by the chart version IRI of its canonical RDF.
//!
//! A plan may declare its result type as `vt:result="{ booked: bool }"` on its root and its
//! needed capabilities as a root `vt:grant`. Self-modification of a running virtual actor goes
//! through [`propose_self_modification`].

use std::collections::BTreeSet;

use vertep_core::compile::CompiledChart;
use vertep_core::ir::{Block, Chart, ExecKind, Extension, ExtensionValue, State, VT_NS};
use vertep_core::types::{BasicTyper, ChartTypes, Declarations, Type, check_chart, parse_type};
use vertep_dm_cel::{LoadOptions, Policy, TypeEnv, function_iris};
use vertep_rdf::ChartVersion;
use vertep_runtime::chart::{element_attributes, parse_duration};
use vertep_runtime::{Address, MigrateError, Migration, Node};

use crate::grant::{self, Grant};
use crate::json::sha256;

/// The serialization of a submitted plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanFormat {
    /// The YAML chart surface.
    Yaml,
    /// The RDF projection in Turtle.
    Turtle,
}

impl PlanFormat {
    /// The format named `s` (`yaml`, `turtle`, `rdf`), YAML by default.
    pub fn parse(s: Option<&str>) -> PlanFormat {
        match s.map(str::trim) {
            Some("turtle" | "rdf" | "ttl") => PlanFormat::Turtle,
            _ => PlanFormat::Yaml,
        }
    }
}

/// The class of a plan failure, as carried by `error.plan`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanErrorClass {
    /// The plan does not parse.
    Parse,
    /// The plan violates the IR checks, the shapes or the plan restrictions.
    Invalid,
    /// An execution bound was exceeded.
    Bounds,
    /// The done data does not have the plan's result type.
    Result,
}

impl PlanErrorClass {
    /// The class name used in events.
    pub fn as_str(self) -> &'static str {
        match self {
            PlanErrorClass::Parse => "parse",
            PlanErrorClass::Invalid => "invalid",
            PlanErrorClass::Bounds => "bounds",
            PlanErrorClass::Result => "result",
        }
    }
}

/// A rejected plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError {
    /// The class.
    pub class: PlanErrorClass,
    /// The reasons found (`datamodel`, `ungranted-function`, `ir`, `shacl`, `type`), first
    /// first.
    pub reasons: Vec<String>,
    /// Every diagnostic or violation.
    pub violations: Vec<String>,
    /// The SHA-256 of the submitted text.
    pub content_hash: String,
}

impl core::fmt::Display for PlanError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "plan rejected ({}): {}",
            self.class.as_str(),
            self.violations.join("; ")
        )
    }
}

impl std::error::Error for PlanError {}

/// An accepted plan.
#[derive(Clone, Debug)]
pub struct AcceptedPlan {
    /// The chart.
    pub chart: Chart,
    /// Its content-addressed version IRI.
    pub version: ChartVersion,
    /// The SHA-256 of the submitted text.
    pub content_hash: String,
    /// The declared result type, with the plan's declarations.
    pub result: Option<(Declarations, Type)>,
    /// The capabilities the plan declares it needs.
    pub needs: Option<Grant>,
}

fn attr<'a>(exts: &'a [Extension], local: &str) -> Option<&'a str> {
    exts.iter().find_map(|e| match &e.value {
        ExtensionValue::Attribute(v) if e.ns == VT_NS && e.local == local => Some(v.as_str()),
        _ => None,
    })
}

/// Parses `text` into the IR.
///
/// # Errors
///
/// [`PlanError`] with the class `parse` and the diagnostics.
// @tg: implements:: [[openspec:plan-charts#Plan Submission]]
pub fn parse(text: &str, format: PlanFormat) -> Result<Chart, PlanError> {
    let fail = |violations: Vec<String>| PlanError {
        class: PlanErrorClass::Parse,
        reasons: vec!["parse".into()],
        violations,
        content_hash: sha256(text.as_bytes()),
    };
    match format {
        PlanFormat::Yaml => vertep_yaml::read(text, &vertep_yaml::ReadOptions::lower_only())
            .map_err(|e| fail(e.diagnostics.iter().map(ToString::to_string).collect())),
        PlanFormat::Turtle => {
            vertep_rdf::read_chart(text.as_bytes(), vertep_rdf::RdfFormat::Turtle)
                .map(|r| r.chart)
                .map_err(|e| fail(vec![format!("{e:?}")]))
        }
    }
}

struct Refs<'a> {
    granted: &'a BTreeSet<String>,
    ungranted: BTreeSet<String>,
    scripts: bool,
}

impl Refs<'_> {
    fn text(&mut self, t: &str) {
        for f in function_iris(t) {
            if !self.granted.contains(&f) {
                self.ungranted.insert(f);
            }
        }
    }

    fn exts(&mut self, exts: &[Extension]) {
        for e in exts {
            match &e.value {
                ExtensionValue::Attribute(v) => self.text(v),
                ExtensionValue::Element(xml) if e.ns == VT_NS && e.local == "call" => {
                    let name = element_attributes(xml)
                        .get("name")
                        .cloned()
                        .unwrap_or_default();
                    if !self.granted.contains(&name) {
                        self.ungranted.insert(name);
                    }
                }
                ExtensionValue::Element(xml) => self.text(xml),
            }
        }
    }

    fn opt(&mut self, e: Option<&String>) {
        if let Some(e) = e {
            self.text(e);
        }
    }

    fn params(&mut self, params: &[vertep_core::ir::Param]) {
        for p in params {
            self.opt(p.expr.as_ref());
            self.opt(p.location.as_ref());
        }
    }

    fn block(&mut self, b: &Block) {
        for e in b {
            self.exts(&e.extensions);
            match &e.kind {
                ExecKind::Script(_) => self.scripts = true,
                ExecKind::Extension(x) => self.exts(std::slice::from_ref(x)),
                ExecKind::Assign { location, expr, .. } => {
                    self.text(location);
                    self.opt(expr.as_ref());
                }
                ExecKind::If { branches } => {
                    for br in branches {
                        self.opt(br.cond.as_ref());
                        self.block(&br.body);
                    }
                }
                ExecKind::Foreach { array, body, .. } => {
                    self.text(array);
                    self.block(body);
                }
                ExecKind::Send(send) => {
                    for x in [
                        &send.eventexpr,
                        &send.targetexpr,
                        &send.typeexpr,
                        &send.idlocation,
                        &send.delayexpr,
                    ] {
                        self.opt(x.as_ref());
                    }
                    self.params(&send.params);
                    if let Some(c) = &send.content {
                        self.opt(c.expr.as_ref());
                    }
                }
                ExecKind::Cancel { sendidexpr, .. } => self.opt(sendidexpr.as_ref()),
                ExecKind::Log { expr, .. } => self.opt(expr.as_ref()),
                ExecKind::Raise { .. } => {}
            }
        }
    }

    fn state(&mut self, s: &State) {
        self.exts(&s.extensions);
        for d in &s.data {
            self.opt(d.expr.as_ref());
        }
        for b in s.onentry.iter().chain(s.onexit.iter()) {
            self.block(b);
        }
        for t in &s.transitions {
            self.exts(&t.extensions);
            self.opt(t.cond.as_ref());
            self.block(&t.body);
        }
        for i in &s.invokes {
            self.exts(&i.extensions);
            self.params(&i.params);
            if let Some(c) = &i.content {
                self.opt(c.expr.as_ref());
            }
            self.block(&i.finalize);
        }
        if let Some(d) = &s.donedata {
            self.params(&d.params);
            if let Some(c) = &d.content {
                self.opt(c.expr.as_ref());
            }
        }
        for c in &s.children {
            self.state(c);
        }
    }
}

/// The CEL type environment of a type-checked chart (context fields under `ctx`, declared
/// events under `ev.data`): the projection every host instantiates CEL charts with,
/// [`vertep_dm_cel::type_env`].
pub fn cel_env(types: &ChartTypes) -> TypeEnv {
    vertep_dm_cel::type_env(types)
}

/// Validates `chart` as a plan: the IR well-formedness checks, the SHACL chart shapes and the
/// plan restrictions, with `functions` the granted function set.
///
/// # Errors
///
/// [`PlanError`] with the class `invalid`, every reason and every violation.
// @tg: implements:: [[openspec:plan-charts#Plan Validation]]
// @tg: implements:: [[openspec:plan-charts#CEL-only Plans]]
pub fn validate(
    chart: &Chart,
    functions: &BTreeSet<String>,
    content_hash: &str,
) -> Result<(), PlanError> {
    let mut reasons: Vec<String> = Vec::new();
    let mut violations: Vec<String> = Vec::new();
    let reason = |r: &str, reasons: &mut Vec<String>| {
        if !reasons.iter().any(|x| x == r) {
            reasons.push(r.to_owned());
        }
    };
    let cel = chart.datamodel == vertep_dm_cel::CEL;
    if !cel && chart.datamodel != vertep_core::null::NULL_DATAMODEL {
        reason("datamodel", &mut reasons);
        violations.push(format!(
            "a plan must use the cel or null datamodel, not {}",
            chart.datamodel
        ));
    }
    let mut refs = Refs {
        granted: functions,
        ungranted: BTreeSet::new(),
        scripts: chart.script.is_some(),
    };
    refs.exts(&chart.extensions);
    for s in &chart.states {
        refs.state(s);
    }
    if refs.scripts {
        reason("datamodel", &mut reasons);
        violations.push("a plan must not contain <script>".into());
    }
    if !refs.ungranted.is_empty() {
        reason("ungranted-function", &mut reasons);
        for f in &refs.ungranted {
            violations.push(format!("the function {f} is not granted"));
        }
    }
    if let Err(v) = CompiledChart::new(chart.clone()) {
        reason("ir", &mut reasons);
        violations.extend(v.iter().map(|x| format!("{x}")));
    }
    let report = vertep_rdf::validate(&vertep_rdf::project(chart).graph);
    if !report.conforms() {
        reason("shacl", &mut reasons);
        violations.extend(report.results.iter().map(ToString::to_string));
    }
    if cel && !refs.scripts {
        match check_chart(chart, &BasicTyper::for_datamodel(&chart.datamodel)) {
            Err(diags) => {
                reason("type", &mut reasons);
                violations.extend(
                    diags
                        .iter()
                        .map(|d| format!("{}: {}", d.location, d.message)),
                );
            }
            Ok(types) => {
                let opts = LoadOptions {
                    env: cel_env(&types),
                    policy: Policy::CelOnly,
                    ..LoadOptions::default()
                };
                if let Err(e) = vertep_dm_cel::load(chart, &opts) {
                    for x in e.0 {
                        let text = x.to_string();
                        if functions.iter().any(|f| text.contains(f.as_str())) {
                            continue;
                        }
                        if !function_iris(&text).is_empty() {
                            continue; // already reported as ungranted
                        }
                        reason("type", &mut reasons);
                        violations.push(text);
                    }
                }
            }
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(PlanError {
            class: PlanErrorClass::Invalid,
            reasons,
            violations,
            content_hash: content_hash.to_owned(),
        })
    }
}

/// Parses, validates and content-addresses a plan.
///
/// # Errors
///
/// [`PlanError`] with the class `parse` or `invalid`.
// @tg: implements:: [[openspec:plan-charts#Content-addressed Plans]]
pub fn submit(
    text: &str,
    format: PlanFormat,
    functions: &BTreeSet<String>,
) -> Result<AcceptedPlan, PlanError> {
    let content_hash = sha256(text.as_bytes());
    let chart = parse(text, format)?;
    validate(&chart, functions, &content_hash)?;
    let version = vertep_rdf::chart_version(&chart);
    let result = attr(&chart.extensions, "result").and_then(|t| {
        let decls = vertep_core::types::chart::declarations(&chart)
            .and_then(Result::ok)
            .unwrap_or_default();
        parse_type(t).ok().map(|ty| (decls, ty))
    });
    let needs = grant::declared(&chart.extensions);
    Ok(AcceptedPlan {
        chart,
        version,
        content_hash,
        result,
        needs,
    })
}

/// The execution bounds of a plan, declared by the invoking `<invoke>`: `vt:maxMacrosteps`,
/// `vt:maxDuration` (a durable timer) and `vt:maxSends`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanBounds {
    /// Macrosteps the plan may run.
    pub max_macrosteps: u64,
    /// Wall-clock duration in milliseconds.
    pub max_duration_ms: Option<i64>,
    /// Outgoing sends (to anything but the plan itself).
    pub max_sends: u64,
}

impl Default for PlanBounds {
    fn default() -> Self {
        PlanBounds {
            max_macrosteps: 1_000,
            max_duration_ms: None,
            max_sends: 100,
        }
    }
}

impl PlanBounds {
    /// The bounds declared on an `<invoke>`.
    // @tg: implements:: [[openspec:plan-charts#Plan Execution Bounds]]
    pub fn of(exts: &[Extension]) -> PlanBounds {
        let d = PlanBounds::default();
        PlanBounds {
            max_macrosteps: attr(exts, "maxMacrosteps")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(d.max_macrosteps),
            max_duration_ms: attr(exts, "maxDuration").and_then(parse_duration),
            max_sends: attr(exts, "maxSends")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(d.max_sends),
        }
    }
}

/// Why a self-modification proposal was rejected.
#[derive(Debug)]
pub enum SelfModError {
    /// The new version failed validation.
    Invalid(PlanError),
    /// The new version could not be registered.
    Register(String),
    /// The migration procedure refused (unmapped configurations, wrong version, ...).
    Migrate(MigrateError),
    /// The actor has no state.
    NotStarted(String),
}

impl core::fmt::Display for SelfModError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SelfModError::Invalid(e) => write!(f, "{e}"),
            SelfModError::Register(e) => write!(f, "cannot register the new version: {e}"),
            SelfModError::Migrate(e) => write!(f, "{e}"),
            SelfModError::NotStarted(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SelfModError {}

/// Proposes a new version of virtual actor `actor`'s own chart with a migration from its
/// current version. The new chart is validated (IR checks and shapes) and registered under its
/// content address; `migration(from, to)` builds the migration, which goes through the
/// chart-versioning migration procedure ([`Node::migrate`]). Acceptance is the migration
/// journal entry; a rejection is journaled by delivering `vt.self-modification.rejected` to the
/// actor. Nothing else can change a running actor's chart.
///
/// # Errors
///
/// [`SelfModError`]; the actor then stays on its current version.
// @tg: implements:: [[openspec:plan-charts#Self-modification Through Versioning]]
pub fn propose_self_modification(
    node: &mut Node,
    actor: &Address,
    chart: Chart,
    migration: impl FnOnce(ChartVersion, ChartVersion) -> Migration,
) -> Result<u64, SelfModError> {
    let reject = |node: &mut Node, reason: String| {
        let ev = vertep_core::datamodel::Event {
            name: crate::vocab::SELF_MODIFICATION_REJECTED_EVENT.into(),
            data: crate::json::obj([("reason", crate::json::s(reason))]),
            ..Default::default()
        };
        let _ = node.send(actor, ev);
        node.run_until_idle();
    };
    let current = node
        .state(actor)
        .map_err(|e| SelfModError::NotStarted(e.to_string()))?;
    let hash = sha256(vertep_rdf::canonical_nquads(&chart).as_bytes());
    let mut violations = Vec::new();
    if let Err(v) = CompiledChart::new(chart.clone()) {
        violations.extend(v.iter().map(|x| format!("{x}")));
    }
    let report = vertep_rdf::validate(&vertep_rdf::project(&chart).graph);
    violations.extend(report.results.iter().map(ToString::to_string));
    if !violations.is_empty() {
        let e = PlanError {
            class: PlanErrorClass::Invalid,
            reasons: vec!["ir".into()],
            violations,
            content_hash: hash,
        };
        reject(node, e.to_string());
        return Err(SelfModError::Invalid(e));
    }
    let to = match node.register_chart(chart) {
        Ok(v) => v,
        Err(e) => {
            reject(node, e.to_string());
            return Err(SelfModError::Register(e.to_string()));
        }
    };
    let from = ChartVersion::parse(&current.version)
        .ok_or_else(|| SelfModError::NotStarted(format!("bad version {}", current.version)))?;
    let m = migration(from, to);
    let iri = m.iri.clone();
    node.register_migration(m);
    match node.migrate(actor, &iri) {
        Ok(p) => Ok(p),
        Err(e) => {
            reject(node, e.to_string());
            Err(SelfModError::Migrate(e))
        }
    }
}
