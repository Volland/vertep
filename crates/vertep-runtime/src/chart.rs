//! Chart versions, the chart registry, `vt:` extension lowering and migrations.
//!
//! A chart version is the content address of the chart as written (`vertep_rdf::chart_version`
//! over the IR before lowering), so the same chart loaded from SCXML or YAML has one version.
//! The registry keeps every version by IRI, the versions of each chart type in registration
//! order (the last is the latest) and the declared migrations.
//!
//! Before compiling, [`lower`] rewrites the runtime's `vt:` extensions into plain SCXML:
//!
//! - `<vt:request target=".." event=".." timeout="5s" reply="s1" ontimeout="s2"/>` in a state:
//!   a `<send>` of type [`vocab::REQUEST_SEND_TYPE`] on entry, a generated waiting substate, a
//!   transition on `vt:reply` and one on `error.timeout`. The runtime assigns the correlation id
//!   and journals the timeout as a durable timer.
//! - `<vt:reminder name=".." period=".." event=".."/>` (or `cancel="true"`) in executable
//!   content: a `<send>` of type [`vocab::REMINDER_SEND_TYPE`].
//! - `<vt:migrate to=".." migration=".."/>` in executable content: a `<send>` of type
//!   [`vocab::MIGRATE_SEND_TYPE`], handled as a self-migration request.
//!
//! `vt:defer` (on states), `vt:unhandled` (on `<scxml>`) and `vt:restart*` (on `<invoke>`) are
//! read from the IR without rewriting it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use vertep_core::compile::{CompiledChart, ROOT, StateIdx};
use vertep_core::datamodel::Value;
use vertep_core::ir::{
    Block, Chart, Exec, ExecKind, Extension, ExtensionValue, Send as SendElem, State, StateElement,
    Transition, VT_NS,
};
use vertep_core::kernel::History;
use vertep_core::session::Session;
use vertep_rdf::ChartVersion;

use crate::vocab;

/// What to do with an external event that enables no transition and is not deferred.
// @tg: implements:: [[openspec:messaging#Unhandled Event Policy]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum UnhandledPolicy {
    /// Drop it, as the SCXML algorithm does (default for plain sessions).
    Discard,
    /// Record it as a `vt:DeadLetter` (default for virtual actors).
    DeadLetter,
    /// Raise `error.unhandled` in the chart with the event as data.
    Error,
}

impl UnhandledPolicy {
    /// The name used in `vt:unhandled` and in journal entries.
    pub fn as_str(self) -> &'static str {
        match self {
            UnhandledPolicy::Discard => "discard",
            UnhandledPolicy::DeadLetter => "dead-letter",
            UnhandledPolicy::Error => "error",
        }
    }

    /// The policy named `s`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "discard" => Some(UnhandledPolicy::Discard),
            "dead-letter" => Some(UnhandledPolicy::DeadLetter),
            "error" => Some(UnhandledPolicy::Error),
            _ => None,
        }
    }
}

/// When an invoked actor is restarted after it fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartMode {
    /// Never (the SCXML default).
    Never,
    /// After a failure.
    OnFailure,
    /// After a failure and after a normal completion, which then raises no `done.invoke`.
    Always,
}

/// A `vt:restart` supervision policy of an `<invoke>`.
// @tg: implements:: [[openspec:actor-model#Supervision With Restart]]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestartPolicy {
    /// When to restart.
    pub mode: RestartMode,
    /// Restarts allowed within the window; one more failure gives up.
    pub limit: u32,
    /// The window, in milliseconds.
    pub window_ms: i64,
    /// The first backoff; the n-th restart waits `base * 2^(n-1)`.
    pub backoff_ms: i64,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy {
            mode: RestartMode::Never,
            limit: 3,
            window_ms: 60_000,
            backoff_ms: 100,
        }
    }
}

impl RestartPolicy {
    /// The backoff before the `n`-th restart (1-based).
    pub fn backoff(&self, n: u32) -> i64 {
        self.backoff_ms
            .saturating_mul(1i64 << n.saturating_sub(1).min(30))
    }
}

/// Parses a duration: a number followed by `ms`, `s`, `min`, `h` or `d`, or a bare number of
/// milliseconds.
pub fn parse_duration(text: &str) -> Option<i64> {
    let t = text.trim();
    let units: [(&str, f64); 5] = [
        ("ms", 1.0),
        ("min", 60_000.0),
        ("s", 1_000.0),
        ("h", 3_600_000.0),
        ("d", 86_400_000.0),
    ];
    for (suffix, factor) in units {
        if let Some(n) = t.strip_suffix(suffix) {
            let v: f64 = n.trim().parse().ok()?;
            return (v >= 0.0).then(|| (v * factor).round() as i64);
        }
    }
    t.parse::<i64>().ok().filter(|v| *v >= 0)
}

/// The attributes of the start tag of an extension element's serialized XML.
pub fn element_attributes(xml: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(start) = xml.find('<') else {
        return out;
    };
    let rest = &xml[start + 1..];
    let end = rest.find('>').unwrap_or(rest.len());
    let tag = rest[..end].trim_end_matches('/');
    let mut chars = tag.char_indices().peekable();
    // skip the element name
    while let Some((_, c)) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        chars.next();
    }
    let bytes = tag;
    let mut i = chars.peek().map_or(bytes.len(), |(i, _)| *i);
    while i < bytes.len() {
        let rest = &bytes[i..];
        let trimmed = rest.trim_start();
        i += rest.len() - trimmed.len();
        let Some(eq) = trimmed.find('=') else { break };
        let name = trimmed[..eq].trim().to_owned();
        let after = trimmed[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            break;
        };
        let body = &after[1..];
        let Some(close) = body.find(q) else { break };
        out.insert(
            name.rsplit(':').next().unwrap_or(&name).to_owned(),
            unescape(&body[..close]),
        );
        let consumed = (trimmed.len() - after.len()) + 1 + close + 1;
        i += consumed;
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn vt_attr<'a>(exts: &'a [Extension], local: &str) -> Option<&'a str> {
    exts.iter().find_map(|e| match &e.value {
        ExtensionValue::Attribute(v) if e.ns == VT_NS && e.local == local => Some(v.as_str()),
        _ => None,
    })
}

fn vt_elements<'a>(
    exts: &'a [Extension],
    local: &'a str,
) -> impl Iterator<Item = BTreeMap<String, String>> + 'a {
    exts.iter().filter_map(move |e| match &e.value {
        ExtensionValue::Element(xml) if e.ns == VT_NS && e.local == local => {
            Some(element_attributes(xml))
        }
        _ => None,
    })
}

/// Why a chart's `vt:` extensions could not be lowered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LowerError(pub String);

impl core::fmt::Display for LowerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid vt: extension: {}", self.0)
    }
}

/// The send type of a `vt:request` with an optional timeout.
// @tg: implements:: [[openspec:messaging#Request Timeout]]
pub fn request_type(timeout_ms: Option<i64>) -> String {
    match timeout_ms {
        Some(t) => format!("{};timeout={t}", vocab::REQUEST_SEND_TYPE),
        None => vocab::REQUEST_SEND_TYPE.to_owned(),
    }
}

/// The parameters of a `vt:` send type: `base;k=v;k2=v2` gives `(base, {k: v, k2: v2})`.
pub fn split_type(type_: &str) -> (&str, BTreeMap<String, String>) {
    let mut parts = type_.split(';');
    let base = parts.next().unwrap_or_default();
    let params = parts
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_owned(), v.to_owned())
        })
        .collect();
    (base, params)
}

/// Rewrites the runtime's `vt:` extensions into plain SCXML (see the module docs), returning
/// the lowered chart and the send types it introduced.
///
/// # Errors
///
/// [`LowerError`] for a `vt:request` without `event` or with a malformed timeout, or a
/// `vt:reminder` without `name`.
// @tg: implements:: [[openspec:messaging#Non-Blocking Request]]
pub fn lower(chart: &Chart) -> Result<(Chart, BTreeSet<String>), LowerError> {
    let mut out = chart.clone();
    let mut types = BTreeSet::new();
    for s in &mut out.states {
        lower_state(s, &mut types)?;
    }
    Ok((out, types))
}

fn lower_state(s: &mut State, types: &mut BTreeSet<String>) -> Result<(), LowerError> {
    let requests: Vec<_> = vt_elements(&s.extensions, "request").collect();
    for (k, r) in requests.iter().enumerate() {
        let event = r
            .get("event")
            .ok_or_else(|| LowerError(format!("vt:request in state {} has no event", s.id)))?;
        let timeout = match r.get("timeout") {
            Some(t) => Some(parse_duration(t).ok_or_else(|| {
                LowerError(format!("vt:request in state {}: bad timeout {t:?}", s.id))
            })?),
            None => None,
        };
        let ty = request_type(timeout);
        types.insert(ty.clone());
        let send = SendElem {
            event: Some(event.clone()),
            target: r.get("target").cloned(),
            type_: Some(ty),
            ..SendElem::default()
        };
        s.onentry
            .push(vec![Exec::new(ExecKind::Send(Box::new(send)))]);
        if k == 0 && s.element == StateElement::State && s.children.is_empty() {
            s.children.push(State::new(
                format!("{}.vt-waiting", s.id),
                StateElement::State,
            ));
        }
        if let Some(target) = r.get("reply") {
            s.transitions.push(Transition {
                events: vec![vocab::REPLY_EVENT.to_owned()],
                targets: vec![target.clone()],
                ..Transition::default()
            });
        }
        if let Some(target) = r.get("ontimeout") {
            s.transitions.push(Transition {
                events: vec![vocab::TIMEOUT_EVENT.to_owned()],
                targets: vec![target.clone()],
                ..Transition::default()
            });
        }
    }
    for b in s.onentry.iter_mut().chain(s.onexit.iter_mut()) {
        lower_block(b, types)?;
    }
    for t in &mut s.transitions {
        lower_block(&mut t.body, types)?;
    }
    for c in &mut s.children {
        lower_state(c, types)?;
    }
    Ok(())
}

fn lower_block(b: &mut Block, types: &mut BTreeSet<String>) -> Result<(), LowerError> {
    for e in b.iter_mut() {
        match &mut e.kind {
            ExecKind::If { branches } => {
                for br in branches {
                    lower_block(&mut br.body, types)?;
                }
            }
            ExecKind::Foreach { body, .. } => lower_block(body, types)?,
            ExecKind::Extension(x) if x.ns == VT_NS => {
                let ExtensionValue::Element(xml) = &x.value else {
                    continue;
                };
                let a = element_attributes(xml);
                let send = match x.local.as_str() {
                    "reminder" => {
                        let name = a
                            .get("name")
                            .ok_or_else(|| LowerError("vt:reminder has no name".into()))?;
                        let ty = if a.get("cancel").is_some_and(|c| c == "true") {
                            format!("{};name={name};cancel=true", vocab::REMINDER_SEND_TYPE)
                        } else {
                            let period = a
                                .get("period")
                                .and_then(|p| parse_duration(p))
                                .filter(|p| *p > 0)
                                .ok_or_else(|| {
                                    LowerError(format!("vt:reminder {name}: bad period"))
                                })?;
                            format!("{};name={name};period={period}", vocab::REMINDER_SEND_TYPE)
                        };
                        SendElem {
                            event: Some(a.get("event").cloned().unwrap_or_else(|| name.clone())),
                            type_: Some(ty),
                            ..SendElem::default()
                        }
                    }
                    "migrate" => {
                        let to = a
                            .get("to")
                            .ok_or_else(|| LowerError("vt:migrate has no `to`".into()))?;
                        let m = a.get("migration").cloned().unwrap_or_default();
                        SendElem {
                            event: Some("vt:migrate".into()),
                            type_: Some(format!(
                                "{};to={to};migration={m}",
                                vocab::MIGRATE_SEND_TYPE
                            )),
                            ..SendElem::default()
                        }
                    }
                    _ => continue,
                };
                if let Some(t) = &send.type_ {
                    types.insert(t.clone());
                }
                e.kind = ExecKind::Send(Box::new(send));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A registered chart version: the IR as written, the compiled lowered chart and what the
/// runtime reads from its `vt:` extensions.
pub struct ChartInfo {
    /// The version IRI (content address of [`ChartInfo::ir`]).
    pub version: ChartVersion,
    /// The chart type this version was registered under, if any.
    pub chart_type: Option<String>,
    /// The chart as written.
    pub ir: Chart,
    /// The lowered, compiled chart that runs.
    pub compiled: CompiledChart,
    /// The `vt:` send types the lowering introduced.
    pub vt_send_types: BTreeSet<String>,
    /// `vt:defer` descriptors per state index.
    pub defers: BTreeMap<StateIdx, Vec<String>>,
    /// The declared `vt:unhandled` policy.
    pub unhandled: Option<UnhandledPolicy>,
}

impl std::fmt::Debug for ChartInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartInfo")
            .field("version", &self.version.as_str())
            .field("chart_type", &self.chart_type)
            .finish_non_exhaustive()
    }
}

/// Why a chart could not be registered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterError {
    /// The chart type is not an absolute IRI.
    BadType(String),
    /// A `vt:` extension is malformed.
    Lower(LowerError),
    /// The lowered chart failed validation.
    Invalid(Vec<String>),
    /// An `<invoke>` targets a virtual actor, which is never invokable.
    InvokesVirtualActor {
        /// The invoking state.
        state: String,
        /// The `src` naming a virtual actor.
        src: String,
    },
}

impl core::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RegisterError::BadType(t) => write!(f, "chart type {t:?} is not an absolute IRI"),
            RegisterError::Lower(e) => write!(f, "{e}"),
            RegisterError::Invalid(v) => write!(f, "chart is invalid: {}", v.join("; ")),
            RegisterError::InvokesVirtualActor { state, src } => write!(
                f,
                "state {state} invokes the virtual actor {src}; virtual actors are not invokable"
            ),
        }
    }
}

impl std::error::Error for RegisterError {}

impl ChartInfo {
    /// Lowers, compiles and analyses `chart`.
    ///
    /// # Errors
    ///
    /// [`RegisterError`] when lowering or validation fails.
    pub fn new(chart: Chart, chart_type: Option<String>) -> Result<Self, RegisterError> {
        let version = vertep_rdf::chart_version(&chart);
        let (lowered, vt_send_types) = lower(&chart).map_err(RegisterError::Lower)?;
        let compiled = CompiledChart::new(lowered)
            .map_err(|v| RegisterError::Invalid(v.iter().map(|x| format!("{x:?}")).collect()))?;
        let mut defers = BTreeMap::new();
        collect_defers(&compiled.chart().states, &compiled, &mut defers);
        let unhandled = vt_attr(&chart.extensions, "unhandled").and_then(UnhandledPolicy::parse);
        Ok(ChartInfo {
            version,
            chart_type,
            ir: chart,
            compiled,
            vt_send_types,
            defers,
            unhandled,
        })
    }

    /// The event descriptors deferred by the states active in `session`.
    // @tg: implements:: [[openspec:messaging#Deferred Events]]
    pub fn deferred(&self, session: &Session) -> Vec<String> {
        let mut out = Vec::new();
        for (idx, ds) in &self.defers {
            if session.configuration.contains(*idx) {
                out.extend(ds.iter().cloned());
            }
        }
        out
    }

    /// The `vt:restart` policy of the `index`-th `<invoke>` of state `state`.
    pub fn restart_policy(&self, state: &str, index: usize) -> RestartPolicy {
        let mut p = RestartPolicy::default();
        let Some(inv) = find_state(&self.ir.states, state).and_then(|s| s.invokes.get(index))
        else {
            return p;
        };
        let x = &inv.extensions;
        p.mode = match vt_attr(x, "restart") {
            Some("on-failure") => RestartMode::OnFailure,
            Some("always") => RestartMode::Always,
            _ => RestartMode::Never,
        };
        if let Some(l) = vt_attr(x, "restart-limit").and_then(|v| v.trim().parse().ok()) {
            p.limit = l;
        }
        if let Some(w) = vt_attr(x, "restart-window").and_then(parse_duration) {
            p.window_ms = w;
        }
        if let Some(b) = vt_attr(x, "restart-backoff").and_then(parse_duration) {
            p.backoff_ms = b;
        }
        p
    }

    /// The active state ids of `session`, in document order.
    pub fn active(&self, session: &Session) -> Vec<String> {
        session.active(&self.compiled)
    }
}

fn collect_defers(
    states: &[State],
    chart: &CompiledChart,
    out: &mut BTreeMap<StateIdx, Vec<String>>,
) {
    for s in states {
        if let Some(d) = vt_attr(&s.extensions, "defer")
            && let Some(i) = chart.index_of(&s.id)
        {
            out.insert(i, d.split_whitespace().map(str::to_owned).collect());
        }
        collect_defers(&s.children, chart, out);
    }
}

/// The state with identifier `id`.
pub fn find_state<'a>(states: &'a [State], id: &str) -> Option<&'a State> {
    for s in states {
        if s.id == id {
            return Some(s);
        }
        if let Some(f) = find_state(&s.children, id) {
            return Some(f);
        }
    }
    None
}

/// Every `<invoke>` with a static `src`, as (state id, src).
pub fn static_invoke_sources(states: &[State], out: &mut Vec<(String, String)>) {
    for s in states {
        for inv in &s.invokes {
            if let Some(src) = &inv.src {
                out.push((s.id.clone(), src.clone()));
            }
        }
        static_invoke_sources(&s.children, out);
    }
}

/// A datamodel transform of a migration.
pub type Transform = Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;

/// A declared migration between two chart versions: a configuration mapping and a datamodel
/// transform. Configurations not listed explicitly map state by state through
/// [`Migration::states`], and a state not listed there maps to the state with the same id in the
/// new version, if it has one (the derived identity mapping).
// @tg: implements:: [[openspec:chart-versioning#Explicit Migration]]
#[derive(Clone)]
pub struct Migration {
    /// The migration's IRI.
    pub iri: String,
    /// The version it migrates from.
    pub from: ChartVersion,
    /// The version it migrates to.
    pub to: ChartVersion,
    /// Explicit configuration mappings (old active state ids to new active state ids).
    pub configurations: Vec<(BTreeSet<String>, BTreeSet<String>)>,
    /// State renames applied to configurations without an explicit mapping.
    pub states: BTreeMap<String, String>,
    /// The datamodel transform.
    pub transform: Transform,
}

impl std::fmt::Debug for Migration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Migration")
            .field("iri", &self.iri)
            .field("from", &self.from.as_str())
            .field("to", &self.to.as_str())
            .finish_non_exhaustive()
    }
}

impl Migration {
    /// A migration with only the derived identity mapping and an identity transform.
    pub fn identity(iri: impl Into<String>, from: ChartVersion, to: ChartVersion) -> Self {
        Migration {
            iri: iri.into(),
            from,
            to,
            configurations: Vec::new(),
            states: BTreeMap::new(),
            transform: Arc::new(Ok),
        }
    }

    /// This migration with an explicit configuration mapping.
    pub fn map_configuration<I: IntoIterator<Item = S>, S: Into<String>>(
        mut self,
        from: I,
        to: I,
    ) -> Self {
        self.configurations.push((
            from.into_iter().map(Into::into).collect(),
            to.into_iter().map(Into::into).collect(),
        ));
        self
    }

    /// This migration with a state rename.
    pub fn rename_state(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.states.insert(from.into(), to.into());
        self
    }

    /// This migration with a datamodel transform.
    pub fn with_transform(
        mut self,
        f: impl Fn(Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Self {
        self.transform = Arc::new(f);
        self
    }

    /// The new configuration for `old`, or `None` when unmapped.
    pub fn map(&self, old: &BTreeSet<String>, to: &ChartInfo) -> Option<BTreeSet<String>> {
        let core: BTreeSet<String> = old.iter().filter(|s| !is_generated(s)).cloned().collect();
        if let Some((_, n)) = self.configurations.iter().find(|(o, _)| *o == core) {
            return Some(n.clone());
        }
        let mut out = BTreeSet::new();
        for s in &core {
            let n = self.states.get(s).unwrap_or(s);
            to.compiled.index_of(n)?;
            out.insert(n.clone());
        }
        Some(out)
    }
}

fn is_generated(id: &str) -> bool {
    id.ends_with(".vt-waiting")
}

/// The configurations reachable in `chart`, ignoring guards (an over-approximation): every
/// legal configuration of its state structure, as sets of state ids.
///
/// # Errors
///
/// The number of configurations found so far when it exceeds `limit`.
// @tg: implements:: [[openspec:chart-versioning#Migration Compatibility Check]]
pub fn reachable_configurations(
    chart: &Chart,
    limit: usize,
) -> Result<Vec<BTreeSet<String>>, usize> {
    let mut all = Vec::new();
    for s in &chart.states {
        if matches!(s.element, StateElement::History(_)) {
            continue;
        }
        all.extend(configs(s, limit)?);
        if all.len() > limit {
            return Err(all.len());
        }
    }
    Ok(all)
}

fn configs(s: &State, limit: usize) -> Result<Vec<BTreeSet<String>>, usize> {
    let kids: Vec<&State> = s
        .children
        .iter()
        .filter(|c| !matches!(c.element, StateElement::History(_)))
        .collect();
    let mut out: Vec<BTreeSet<String>> = Vec::new();
    if kids.is_empty() {
        out.push(BTreeSet::from([s.id.clone()]));
    } else if s.element == StateElement::Parallel {
        out.push(BTreeSet::from([s.id.clone()]));
        for k in kids {
            let sub = configs(k, limit)?;
            let mut next = Vec::with_capacity(out.len() * sub.len());
            for a in &out {
                for b in &sub {
                    let mut c = a.clone();
                    c.extend(b.iter().cloned());
                    next.push(c);
                }
                if next.len() > limit {
                    return Err(next.len());
                }
            }
            out = next;
        }
    } else {
        for k in kids {
            for mut c in configs(k, limit)? {
                c.insert(s.id.clone());
                out.push(c);
            }
            if out.len() > limit {
                return Err(out.len());
            }
        }
    }
    Ok(out)
}

/// The configurations of `from` that `m` leaves unmapped into `to`; empty when the migration
/// passes the compatibility check.
pub fn unmapped_configurations(
    m: &Migration,
    from: &ChartInfo,
    to: &ChartInfo,
) -> Result<Vec<BTreeSet<String>>, usize> {
    Ok(reachable_configurations(&from.ir, 100_000)?
        .into_iter()
        .filter(|c| m.map(c, to).is_none())
        .collect())
}

/// The session under `to` for active state ids `ids`, carrying the counters of `old`: compound
/// states without an active child get their default entry, history and invocations start empty.
pub fn migrated_session(old: &Session, ids: &BTreeSet<String>, to: &ChartInfo) -> Session {
    let c = &to.compiled;
    let mut config = c.empty_set();
    for id in ids {
        if let Some(i) = c.index_of(id) {
            config.insert(i);
            let mut p = c.state(i).parent;
            while let Some(a) = p {
                if a == ROOT {
                    break;
                }
                config.insert(a);
                p = c.state(a).parent;
            }
        }
    }
    complete(c, ROOT, &mut config);
    let mut data = config.clone();
    data.insert(ROOT);
    let mut options = old.options.clone();
    for t in &to.vt_send_types {
        if !options.send_types.contains(t) {
            options.send_types.push(t.clone());
        }
    }
    Session {
        sessionid: old.sessionid.clone(),
        status: old.status,
        configuration: config,
        history: History::new(c.len()),
        internal_queue: Default::default(),
        invocations: Vec::new(),
        data_initialized: data,
        send_counter: old.send_counter,
        invoke_counter: old.invoke_counter,
        macrosteps: old.macrosteps,
        options,
    }
}

fn complete(c: &CompiledChart, s: StateIdx, config: &mut vertep_core::bitset::StateSet) {
    let st = c.state(s);
    if st.children.is_empty() {
        return;
    }
    let parallel = st.kind == vertep_core::ir::StateKind::Parallel;
    let active: Vec<StateIdx> = st
        .children
        .iter()
        .copied()
        .filter(|k| config.contains(*k))
        .collect();
    if parallel {
        for k in st.children.clone() {
            config.insert(k);
            complete(c, k, config);
        }
    } else if active.is_empty() {
        if let Some(&k) = c.default_entry(s).first().or(st.children.first()) {
            config.insert(k);
            let mut p = c.state(k).parent;
            while let Some(a) = p {
                if a == s {
                    break;
                }
                config.insert(a);
                p = c.state(a).parent;
            }
            complete(c, k, config);
        }
    } else {
        for k in active {
            complete(c, k, config);
        }
    }
}

/// Every chart version an actor may run, keyed by version IRI, the versions of each chart type
/// in registration order, and the declared migrations.
// @tg: implements:: [[openspec:chart-versioning#Chart Registry]]
#[derive(Default)]
pub struct ChartRegistry {
    versions: BTreeMap<String, Arc<ChartInfo>>,
    types: BTreeMap<String, Vec<String>>,
    migrations: BTreeMap<String, Migration>,
    /// Addresses bound to a chart type and key without the `<type>/<key>` form (DIDs).
    bindings: BTreeMap<String, (String, String)>,
}

impl std::fmt::Debug for ChartRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChartRegistry")
            .field("versions", &self.versions.keys().collect::<Vec<_>>())
            .field("types", &self.types)
            .finish_non_exhaustive()
    }
}

impl ChartRegistry {
    /// Adds a version (idempotent) and, with a type, makes it the type's latest version.
    pub fn insert(&mut self, info: ChartInfo) -> Arc<ChartInfo> {
        let key = info.version.as_str().to_owned();
        let ty = info.chart_type.clone();
        let info = self
            .versions
            .entry(key.clone())
            .or_insert_with(|| Arc::new(info))
            .clone();
        if let Some(t) = ty {
            let list = self.types.entry(t).or_default();
            list.retain(|v| *v != key);
            list.push(key);
        }
        info
    }

    /// The version `iri`.
    pub fn get(&self, iri: &str) -> Option<Arc<ChartInfo>> {
        self.versions.get(iri).cloned()
    }

    /// The latest version registered for chart type `ty`.
    pub fn latest(&self, ty: &str) -> Option<Arc<ChartInfo>> {
        self.types
            .get(ty)
            .and_then(|l| l.last())
            .and_then(|v| self.get(v))
    }

    /// The versions registered for chart type `ty`, oldest first.
    pub fn versions_of(&self, ty: &str) -> Vec<String> {
        self.types.get(ty).cloned().unwrap_or_default()
    }

    /// Every registered chart type.
    pub fn types(&self) -> impl Iterator<Item = &String> {
        self.types.keys()
    }

    /// The chart type and key of a virtual actor address (`<type>/<key>`), for the longest
    /// registered type that prefixes it, or the type and key the address is bound to.
    pub fn type_of(&self, address: &str) -> Option<(String, String)> {
        if let Some(b) = self.bindings.get(address) {
            return Some(b.clone());
        }
        self.types
            .keys()
            .filter_map(|t| {
                let rest = address.strip_prefix(t.as_str())?.strip_prefix('/')?;
                Some((t, rest))
            })
            .max_by_key(|(t, _)| t.len())
            .and_then(|(t, k)| Some((t.clone(), crate::address::percent_decode(k)?)))
    }

    /// Binds `address` to the virtual actor of `chart_type` with `key`, for addresses that do
    /// not have the `<type>/<key>` form, such as an instance whose address is its DID.
    pub fn bind(&mut self, address: &str, chart_type: &str, key: &str) {
        self.bindings
            .insert(address.to_owned(), (chart_type.to_owned(), key.to_owned()));
    }

    /// Removes version `iri`; the caller has checked that no non-final actor uses it.
    pub fn remove(&mut self, iri: &str) -> bool {
        for l in self.types.values_mut() {
            l.retain(|v| v != iri);
        }
        self.versions.remove(iri).is_some()
    }

    /// Declares a migration.
    pub fn add_migration(&mut self, m: Migration) {
        self.migrations.insert(m.iri.clone(), m);
    }

    /// The migration `iri`.
    pub fn migration(&self, iri: &str) -> Option<Migration> {
        self.migrations.get(iri).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("100ms"), Some(100));
        assert_eq!(parse_duration("5s"), Some(5_000));
        assert_eq!(parse_duration("10min"), Some(600_000));
        assert_eq!(parse_duration("1h"), Some(3_600_000));
        assert_eq!(parse_duration("250"), Some(250));
        assert_eq!(parse_duration("x"), None);
    }

    #[test]
    fn attributes_of_an_extension_element() {
        let a = element_attributes(
            r#"<vt:request xmlns:vt="https://w3id.org/vertep/vt#" target="urn:b" event='price.get' timeout="5s"/>"#,
        );
        assert_eq!(a.get("target").map(String::as_str), Some("urn:b"));
        assert_eq!(a.get("event").map(String::as_str), Some("price.get"));
        assert_eq!(a.get("timeout").map(String::as_str), Some("5s"));
        assert_eq!(
            split_type("x;a=1;b=2").1.get("b").map(String::as_str),
            Some("2")
        );
    }

    #[test]
    fn backoff_doubles() {
        let p = RestartPolicy {
            backoff_ms: 100,
            ..RestartPolicy::default()
        };
        assert_eq!((p.backoff(1), p.backoff(2), p.backoff(3)), (100, 200, 400));
    }
}
