//! Lowering a YAML node tree into the chart IR, with located diagnostics.

use std::collections::HashMap;

use vertep_core::ir::{
    Binding, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, IfBranch, Initial, Invoke, Param, Script, Send, Span, State, StateElement,
    Transition, TransitionType, VT_NS,
};
use vertep_core::types::chart::{
    declarations_element, set_data_type, set_declarations, set_param_type, set_send_type,
};
use vertep_core::types::decl::suggest;
use vertep_core::types::{
    BasicTyper, Declarations, DiagnosticKind, EventDecl, Type, TypeDecl, check_chart,
};
use vertep_core::validate::{ViolationKind, validate};

use crate::node::{Kind, Node, Pos, parse};
use crate::types_yaml::type_from_node;
use crate::{DEFAULT_DATAMODEL, Diagnostic, ErrorKind, LoadError, ReadOptions, SUPPORTED_VERSIONS};

pub(crate) const TOP_KEYS: &[&str] = &[
    "vertep",
    "name",
    "datamodel",
    "binding",
    "initial",
    "types",
    "events",
    "context",
    "script",
    "states",
    "extensions",
];
pub(crate) const STATE_KEYS: &[&str] = &[
    "type",
    "history",
    "generated",
    "initial",
    "initial_element",
    "context",
    "entry",
    "exit",
    "invoke",
    "on",
    "always",
    "transitions",
    "donedata",
    "states",
    "extensions",
];
const TRANSITION_KEYS: &[&str] = &["target", "cond", "type", "actions", "extensions"];
const TRANSITION_LIST_KEYS: &[&str] = &["event", "target", "cond", "type", "actions", "extensions"];
/// The executable content keys; an item has exactly one of them.
pub(crate) const ACTION_KEYS: &[&str] = &[
    "assign",
    "raise",
    "send",
    "log",
    "if",
    "foreach",
    "cancel",
    "script",
    "call",
    "extension",
];
const SEND_KEYS: &[&str] = &[
    "event",
    "eventexpr",
    "target",
    "targetexpr",
    "type",
    "typeexpr",
    "id",
    "idlocation",
    "delay",
    "delayexpr",
    "namelist",
    "params",
    "content",
    "payload_type",
];
const INVOKE_KEYS: &[&str] = &[
    "id",
    "idlocation",
    "type",
    "typeexpr",
    "src",
    "srcexpr",
    "namelist",
    "autoforward",
    "params",
    "content",
    "finalize",
    "extensions",
];
const DATA_KEYS: &[&str] = &["type", "init", "src", "content", "extensions"];
const DATA_LIST_KEYS: &[&str] = &["id", "type", "init", "src", "content", "extensions"];
const EXTENSION_KEYS: &[&str] = &["ns", "local", "attribute", "element", "position"];

/// The canonical XML of a `call` item: a `vt:call` element naming the function.
pub(crate) fn call_xml(name: &str) -> String {
    let mut out = format!("<vt:call xmlns:vt=\"{VT_NS}\" name=\"");
    for c in name.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
    out.push_str("\"/>");
    out
}

pub(crate) fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn index(path: &str, i: usize) -> String {
    format!("{path}[{i}]")
}

fn span(p: Pos) -> Span {
    Span::new(p.line, p.column)
}

/// The keys of a mapping, checked against the allowed set.
struct Fields<'n> {
    entries: Vec<(&'n str, &'n Node, &'n Node)>,
}

impl<'n> Fields<'n> {
    fn get(&self, key: &str) -> Option<&'n Node> {
        self.entries
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, _, v)| *v)
    }
}

pub(crate) struct Lower<'o> {
    pub diags: Vec<Diagnostic>,
    options: ReadOptions<'o>,
    datamodel: String,
    /// IR node position to dotted path.
    paths: HashMap<Pos, String>,
    /// `(target, position, path)` of every state reference.
    targets: Vec<(String, Pos, String)>,
    /// `(id, position, path)` of every state.
    ids: Vec<(String, Pos, String)>,
    /// Declarations read from `types` and `events`, with their positions.
    decls: Option<Declarations>,
    decl_paths: Vec<(Pos, String)>,
}

impl<'o> Lower<'o> {
    pub(crate) fn new(options: ReadOptions<'o>) -> Self {
        Lower {
            diags: Vec::new(),
            options,
            datamodel: DEFAULT_DATAMODEL.to_string(),
            paths: HashMap::new(),
            targets: Vec::new(),
            ids: Vec::new(),
            decls: None,
            decl_paths: Vec::new(),
        }
    }

    fn err(&mut self, kind: ErrorKind, pos: Pos, path: &str, message: impl Into<String>) {
        self.diags.push(Diagnostic {
            kind,
            file: self.options.file.map(str::to_string),
            line: pos.line,
            column: pos.column,
            path: path.to_string(),
            message: message.into(),
        });
    }

    fn schema(&mut self, pos: Pos, path: &str, message: impl Into<String>) {
        self.err(ErrorKind::Schema, pos, path, message);
    }

    fn mark(&mut self, pos: Pos, path: &str) {
        self.paths.entry(pos).or_insert_with(|| path.to_string());
    }

    /// Check a mapping's keys: scalar, unique and among `allowed`.
    fn fields<'n>(
        &mut self,
        v: &'n Node,
        path: &str,
        allowed: &[&str],
        what: &str,
    ) -> Option<Fields<'n>> {
        let Some(entries) = v.as_map() else {
            self.schema(
                v.pos,
                path,
                format!("{what} must be a mapping, found {}", v.describe()),
            );
            return None;
        };
        let mut out: Vec<(&'n str, &'n Node, &'n Node)> = Vec::new();
        for (k, val) in entries {
            let Some(key) = k.scalar() else {
                self.schema(k.pos, path, "mapping keys must be scalars");
                continue;
            };
            if let Some((_, first, _)) = out.iter().find(|(e, _, _)| *e == key) {
                let line = first.pos.line;
                self.schema(
                    k.pos,
                    &join(path, key),
                    format!("duplicate key `{key}` (first at line {line})"),
                );
                continue;
            }
            if !allowed.contains(&key) {
                let mut msg = format!("unknown key `{key}` in {what}");
                if let Some(s) = suggest(key, allowed.iter().copied()) {
                    msg.push_str(&format!("; did you mean `{s}`?"));
                } else {
                    msg.push_str(&format!(" (expected one of: {})", allowed.join(", ")));
                }
                self.schema(k.pos, &join(path, key), msg);
                continue;
            }
            out.push((key, k, val));
        }
        Some(Fields { entries: out })
    }

    /// Any scalar's text; null and collections are errors.
    fn string(&mut self, v: &Node, path: &str) -> Option<String> {
        match &v.kind {
            Kind::Scalar { text, .. } if !v.is_null() => Some(text.clone()),
            _ => {
                self.schema(
                    v.pos,
                    path,
                    format!("expected a string, found {}", v.describe()),
                );
                None
            }
        }
    }

    /// An optional string: null is `None`.
    fn opt_string(&mut self, v: Option<&Node>, path: &str) -> Option<String> {
        let v = v?;
        if v.is_null() {
            return None;
        }
        self.string(v, path)
    }

    /// An optional expression string, checked by the datamodel's parser.
    // @tg: implements:: [[openspec:yaml-format#YAML Uses Expression Strings of Its Datamodel]] {confidence: partial, note: "parsing goes through a pluggable ExpressionChecker; no CEL parser is wired in yet"}
    fn expr(&mut self, v: Option<&Node>, path: &str) -> Option<String> {
        let node = v?;
        let text = self.opt_string(Some(node), path)?;
        if let Some(checker) = self.options.expressions
            && let Err(e) = checker.check(&self.datamodel, &text)
        {
            let plain = matches!(node.kind, Kind::Scalar { plain: true, .. });
            let before = text.get(..e.offset).unwrap_or(&text);
            let col = if before.contains('\n') {
                node.pos.column
            } else {
                node.pos.column
                    + u32::from(!plain)
                    + u32::try_from(before.chars().count()).unwrap_or(0)
            };
            self.err(
                ErrorKind::Expression,
                Pos {
                    line: node.pos.line,
                    column: col,
                },
                path,
                format!("{} datamodel: {}", self.datamodel, e.message),
            );
        }
        Some(text)
    }

    fn bool(&mut self, v: &Node, path: &str) -> bool {
        match v.as_bool() {
            Some(b) => b,
            None => {
                self.schema(
                    v.pos,
                    path,
                    format!("expected true or false, found {}", v.describe()),
                );
                false
            }
        }
    }

    /// A string or a list of strings.
    fn strings(&mut self, v: &Node, path: &str) -> Vec<String> {
        match &v.kind {
            Kind::Seq(items) => items
                .iter()
                .enumerate()
                .filter_map(|(i, n)| self.string(n, &index(path, i)))
                .collect(),
            _ => self.string(v, path).into_iter().collect(),
        }
    }

    /// State references: a string or a list of strings, recorded for the reference check.
    fn refs(&mut self, v: &Node, path: &str) -> Vec<String> {
        let mut out = Vec::new();
        match &v.kind {
            Kind::Seq(items) => {
                for (i, n) in items.iter().enumerate() {
                    let p = index(path, i);
                    if let Some(s) = self.string(n, &p) {
                        self.targets.push((s.clone(), n.pos, p));
                        out.push(s);
                    }
                }
            }
            _ => {
                if let Some(s) = self.string(v, path) {
                    self.targets.push((s.clone(), v.pos, path.to_string()));
                    out.push(s);
                }
            }
        }
        out
    }

    /// `X` (explicit) or `{ value: X, implicit: true }`.
    fn flagged(&mut self, v: &Node, path: &str, allowed: &[&str]) -> Option<(String, bool)> {
        let (value, explicit, at) = if v.as_map().is_some() {
            let f = self.fields(v, path, &["value", "implicit"], "a flagged value")?;
            let Some(val) = f.get("value") else {
                self.schema(v.pos, path, "missing key `value`");
                return None;
            };
            let implicit = match f.get("implicit") {
                Some(n) => self.bool(n, &join(path, "implicit")),
                None => false,
            };
            (self.string(val, &join(path, "value"))?, !implicit, val.pos)
        } else {
            (self.string(v, path)?, true, v.pos)
        };
        if !allowed.is_empty() && !allowed.contains(&value.as_str()) {
            self.schema(
                at,
                path,
                format!("`{value}` is not one of: {}", allowed.join(", ")),
            );
            return None;
        }
        Some((value, explicit))
    }

    fn type_spec(&mut self, v: &Node, path: &str) -> Option<Type> {
        match type_from_node(v) {
            Ok(t) => Some(t),
            Err(errs) => {
                for e in errs {
                    let mut pos = e.node.pos;
                    if let Some(o) = e.offset {
                        let plain = matches!(e.node.kind, Kind::Scalar { plain: true, .. });
                        pos.column += u32::from(!plain) + u32::try_from(o).unwrap_or(0);
                    }
                    self.err(ErrorKind::Type, pos, path, e.message);
                }
                None
            }
        }
    }

    // ============================================================================ document

    pub(crate) fn chart(&mut self, root: &Node) -> Option<Chart> {
        let f = self.fields(root, "", TOP_KEYS, "a chart document")?;
        match f.get("vertep") {
            None => self.err(
                ErrorKind::Version,
                root.pos,
                "vertep",
                format!(
                    "the format version is required: add `vertep: 1` (supported versions: {})",
                    versions()
                ),
            ),
            Some(v) => match v.as_u32() {
                Some(n) if SUPPORTED_VERSIONS.contains(&n) => {}
                _ => {
                    let shown = v.scalar().unwrap_or("?").to_string();
                    self.err(
                        ErrorKind::Version,
                        v.pos,
                        "vertep",
                        format!(
                            "unsupported format version `{shown}`; supported versions: {}",
                            versions()
                        ),
                    );
                }
            },
        }
        let mut chart = Chart {
            span: span(root.pos),
            ..Chart::default()
        };
        match f.get("name") {
            None => self.schema(
                root.pos,
                "name",
                "the chart `name` is required (use null for none)",
            ),
            Some(v) => chart.name = self.opt_string(Some(v), "name"),
        }
        let (dm, explicit) = match f.get("datamodel") {
            Some(v) => self
                .flagged(v, "datamodel", &[])
                .unwrap_or((DEFAULT_DATAMODEL.into(), true)),
            None => (DEFAULT_DATAMODEL.to_string(), false),
        };
        self.datamodel.clone_from(&dm);
        chart.datamodel = dm;
        chart.datamodel_explicit = explicit;
        if let Some(v) = f.get("binding")
            && let Some((b, explicit)) = self.flagged(v, "binding", &["early", "late"])
        {
            chart.binding = if b == "late" {
                Binding::Late
            } else {
                Binding::Early
            };
            chart.binding_explicit = explicit;
        }
        if let Some(v) = f.get("initial") {
            chart.initial = self.refs(v, "initial");
        }
        if let Some(v) = f.get("context") {
            chart.data = self.data_specs(v, "context");
        }
        if let Some(v) = f.get("script") {
            chart.script = self.script(v, "script");
        }
        let types = f.get("types");
        let events = f.get("events");
        if types.is_some() || events.is_some() {
            let d = self.declarations(types, events);
            self.decls = Some(d);
        }
        match f.get("states") {
            None => self.schema(root.pos, "states", "the `states` mapping is required"),
            Some(v) => chart.states = self.states(v, "states"),
        }
        if let Some(v) = f.get("extensions") {
            chart.extensions = self.extensions(v, "extensions");
        }
        if let Some(d) = self.decls.clone() {
            if declarations_element(&chart).is_some() {
                let pos = types.or(events).map_or(root.pos, |n| n.pos);
                self.schema(
                    pos,
                    "types",
                    "a vt:types extension and a types block cannot both be given",
                );
            }
            set_declarations(&mut chart, Some(&d));
        }
        self.mark(root.pos, "");
        Some(chart)
    }

    // @tg: implements:: [[openspec:yaml-format#YAML Types Block]]
    fn declarations(&mut self, types: Option<&Node>, events: Option<&Node>) -> Declarations {
        let mut d = Declarations::default();
        if let Some(t) = types
            && !t.is_null()
            && let Some(f) = self.fields_any(t, "types", "the types block")
        {
            for (name, k, v) in f {
                let p = join("types", &name);
                if let Some(ty) = self.type_spec(v, &p) {
                    d.types.push(TypeDecl {
                        name,
                        ty,
                        span: span(k.pos),
                    });
                    self.decl_paths.push((k.pos, p));
                }
            }
        }
        if let Some(e) = events
            && !e.is_null()
            && let Some(f) = self.fields_any(e, "events", "the events block")
        {
            for (name, k, v) in f {
                let p = join("events", &name);
                let payload = if v.is_null() {
                    None
                } else {
                    self.type_spec(v, &p)
                };
                d.events.push(EventDecl {
                    name,
                    payload,
                    span: span(k.pos),
                });
                self.decl_paths.push((k.pos, p));
            }
        }
        d
    }

    /// A mapping with arbitrary keys (state ids, type names), duplicates reported.
    fn fields_any<'n>(
        &mut self,
        v: &'n Node,
        path: &str,
        what: &str,
    ) -> Option<Vec<(String, &'n Node, &'n Node)>> {
        let Some(entries) = v.as_map() else {
            self.schema(
                v.pos,
                path,
                format!("{what} must be a mapping, found {}", v.describe()),
            );
            return None;
        };
        let mut out: Vec<(String, &'n Node, &'n Node)> = Vec::new();
        for (k, val) in entries {
            let Some(key) = k.scalar() else {
                self.schema(k.pos, path, "mapping keys must be scalars");
                continue;
            };
            if let Some((_, first, _)) = out.iter().find(|(e, _, _)| e == key) {
                let line = first.pos.line;
                self.schema(
                    k.pos,
                    &join(path, key),
                    format!("duplicate key `{key}` (first at line {line})"),
                );
                continue;
            }
            out.push((key.to_string(), k, val));
        }
        Some(out)
    }

    // ============================================================================ states

    fn states(&mut self, v: &Node, path: &str) -> Vec<State> {
        let Some(entries) = v.as_map() else {
            self.schema(
                v.pos,
                path,
                format!(
                    "`states` must be a mapping of state id to state, found {}",
                    v.describe()
                ),
            );
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, val) in entries {
            let Some(id) = k.scalar() else {
                self.schema(k.pos, path, "state ids must be scalars");
                continue;
            };
            let p = join(path, id);
            self.ids.push((id.to_string(), k.pos, p.clone()));
            out.push(self.state(id, k.pos, val, &p));
        }
        out
    }

    // @tg: implements:: [[openspec:yaml-format#States and Hierarchy]]
    fn state(&mut self, id: &str, at: Pos, v: &Node, path: &str) -> State {
        let mut s = State::new(id, StateElement::State);
        s.span = span(at);
        self.mark(at, path);
        if v.is_null() {
            return s;
        }
        let Some(f) = self.fields(v, path, STATE_KEYS, "a state") else {
            return s;
        };
        let history = f.get("history").and_then(|h| {
            let p = join(path, "history");
            match self.string(h, &p).as_deref() {
                Some("shallow") => Some(HistoryType::Shallow),
                Some("deep") => Some(HistoryType::Deep),
                Some(other) => {
                    self.schema(h.pos, &p, format!("`{other}` is not one of: shallow, deep"));
                    None
                }
                None => None,
            }
        });
        if let Some(t) = f.get("type") {
            let p = join(path, "type");
            s.element = match self.string(t, &p).as_deref() {
                Some("state") | None => StateElement::State,
                Some("parallel") => StateElement::Parallel,
                Some("final") => StateElement::Final,
                Some("history") => StateElement::History(history.unwrap_or_default()),
                Some(other) => {
                    self.schema(
                        t.pos,
                        &p,
                        format!("`{other}` is not a state type (state, parallel, final, history)"),
                    );
                    StateElement::State
                }
            };
        }
        if let (Some(h), false) = (
            f.get("history"),
            matches!(s.element, StateElement::History(_)),
        ) {
            self.schema(
                h.pos,
                &join(path, "history"),
                "`history` is only allowed with `type: history`",
            );
        }
        if let Some(g) = f.get("generated") {
            s.id_generated = self.bool(g, &join(path, "generated"));
        }
        if let Some(e) = f.get("extensions") {
            s.extensions = self.extensions(e, &join(path, "extensions"));
        }
        for (key, _, val) in &f.entries {
            let p = join(path, key);
            match *key {
                "initial" => s.initial = self.refs(val, &p),
                "initial_element" => s.initial_element = self.initial_element(val, &p),
                "context" => s.data = self.data_specs(val, &p),
                "entry" => s.onentry = self.blocks(val, &p),
                "exit" => s.onexit = self.blocks(val, &p),
                "invoke" => {
                    s.invokes = match &val.kind {
                        Kind::Seq(items) => items
                            .iter()
                            .enumerate()
                            .map(|(i, n)| self.invoke(n, &index(&p, i)))
                            .collect(),
                        _ => vec![self.invoke(val, &p)],
                    }
                }
                "on" => {
                    if let Some(groups) = self.fields_any(val, &p, "`on`") {
                        for (desc, _, g) in groups {
                            let gp = join(&p, &desc);
                            let events: Vec<String> =
                                desc.split_whitespace().map(str::to_string).collect();
                            if events.is_empty() {
                                self.schema(g.pos, &gp, "an empty event descriptor; use `always` for eventless transitions");
                                continue;
                            }
                            let ts = self.group(g, &events, &gp);
                            s.transitions.extend(ts);
                        }
                    }
                }
                "always" => {
                    let ts = self.group(val, &[], &p);
                    s.transitions.extend(ts);
                }
                "transitions" => {
                    let Some(items) = val.as_seq() else {
                        self.schema(val.pos, &p, "`transitions` must be a list of transitions");
                        continue;
                    };
                    for (i, n) in items.iter().enumerate() {
                        if let Some(t) = self.transition_map(n, &[], &index(&p, i), true) {
                            s.transitions.push(t);
                        }
                    }
                }
                "donedata" => s.donedata = self.donedata(val, &p, &mut s.extensions),
                "states" => s.children = self.states(val, &p),
                _ => {}
            }
        }
        s
    }

    fn initial_element(&mut self, v: &Node, path: &str) -> Option<Initial> {
        let f = self.fields(v, path, &["transition", "extensions"], "an initial element")?;
        let transition = match f.get("transition") {
            Some(t) => self.transition_map(t, &[], &join(path, "transition"), true)?,
            None => {
                self.schema(v.pos, path, "missing key `transition`");
                return None;
            }
        };
        let extensions = match f.get("extensions") {
            Some(e) => self.extensions(e, &join(path, "extensions")),
            None => Vec::new(),
        };
        self.mark(v.pos, path);
        Some(Initial {
            transition,
            extensions,
            span: span(v.pos),
        })
    }

    /// The transitions of one `on` descriptor or of `always`.
    // @tg: implements:: [[openspec:yaml-format#Transitions]]
    fn group(&mut self, v: &Node, events: &[String], path: &str) -> Vec<Transition> {
        let simple = |targets: Vec<String>, pos: Pos| Transition {
            events: events.to_vec(),
            targets,
            span: span(pos),
            ..Transition::default()
        };
        match &v.kind {
            _ if v.is_null() => {
                self.mark(v.pos, path);
                vec![simple(Vec::new(), v.pos)]
            }
            Kind::Scalar { .. } => {
                self.mark(v.pos, path);
                let t = self.refs(v, &join(path, "target"));
                vec![simple(t, v.pos)]
            }
            Kind::Map(_) => self
                .transition_map(v, events, path, false)
                .into_iter()
                .collect(),
            Kind::Seq(items) => {
                if items.iter().all(|n| n.scalar().is_some() && !n.is_null()) {
                    self.mark(v.pos, path);
                    let t = self.refs(v, &join(path, "target"));
                    return vec![simple(t, v.pos)];
                }
                let mut out = Vec::new();
                for (i, n) in items.iter().enumerate() {
                    let p = index(path, i);
                    match &n.kind {
                        _ if n.is_null() => {
                            self.mark(n.pos, &p);
                            out.push(simple(Vec::new(), n.pos));
                        }
                        Kind::Scalar { .. } => {
                            self.mark(n.pos, &p);
                            let t = self.refs(n, &join(&p, "target"));
                            out.push(simple(t, n.pos));
                        }
                        Kind::Map(_) => out.extend(self.transition_map(n, events, &p, false)),
                        Kind::Seq(_) => self.schema(
                            n.pos,
                            &p,
                            "a transition must be a target or a mapping, found a list",
                        ),
                    }
                }
                out
            }
        }
    }

    fn transition_map(
        &mut self,
        v: &Node,
        events: &[String],
        path: &str,
        with_event: bool,
    ) -> Option<Transition> {
        let keys = if with_event {
            TRANSITION_LIST_KEYS
        } else {
            TRANSITION_KEYS
        };
        let f = self.fields(v, path, keys, "a transition")?;
        self.mark(v.pos, path);
        let mut t = Transition {
            events: events.to_vec(),
            span: span(v.pos),
            ..Transition::default()
        };
        if let Some(e) = f.get("event") {
            t.events = match &e.kind {
                Kind::Seq(_) => self.strings(e, &join(path, "event")),
                _ => self
                    .string(e, &join(path, "event"))
                    .map(|s| s.split_whitespace().map(str::to_string).collect())
                    .unwrap_or_default(),
            };
        }
        if let Some(n) = f.get("target") {
            t.targets = self.refs(n, &join(path, "target"));
        }
        t.cond = self.expr(f.get("cond"), &join(path, "cond"));
        if let Some(k) = f.get("type")
            && let Some((kind, explicit)) =
                self.flagged(k, &join(path, "type"), &["external", "internal"])
        {
            t.kind = if kind == "internal" {
                TransitionType::Internal
            } else {
                TransitionType::External
            };
            t.kind_explicit = explicit;
        }
        if let Some(a) = f.get("actions") {
            t.body = self.block(a, &join(path, "actions"));
        }
        if let Some(e) = f.get("extensions") {
            t.extensions = self.extensions(e, &join(path, "extensions"));
        }
        Some(t)
    }

    // ============================================================================ content

    /// `entry` and `exit`: one block (a list of items) or several (a list of lists).
    fn blocks(&mut self, v: &Node, path: &str) -> Vec<Vec<Exec>> {
        match v.as_seq() {
            Some(items) if !items.is_empty() && items.iter().all(|n| n.as_seq().is_some()) => items
                .iter()
                .enumerate()
                .map(|(i, b)| self.block(b, &index(path, i)))
                .collect(),
            _ => vec![self.block(v, path)],
        }
    }

    fn block(&mut self, v: &Node, path: &str) -> Vec<Exec> {
        if v.is_null() {
            return Vec::new();
        }
        let Some(items) = v.as_seq() else {
            self.schema(
                v.pos,
                path,
                format!("expected a list of actions, found {}", v.describe()),
            );
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter_map(|(i, n)| self.item(n, &index(path, i), i))
            .collect()
    }

    // @tg: implements:: [[openspec:yaml-format#Executable Content]]
    fn item(&mut self, v: &Node, path: &str, position: usize) -> Option<Exec> {
        let Some(entries) = v.as_map() else {
            self.schema(
                v.pos,
                path,
                format!(
                    "an action must be a mapping with one key, found {}",
                    v.describe()
                ),
            );
            return None;
        };
        let actions: Vec<&str> = entries
            .iter()
            .filter_map(|(k, _)| k.scalar())
            .filter(|k| ACTION_KEYS.contains(k))
            .collect();
        if actions.len() > 1 {
            self.schema(
                v.pos,
                path,
                format!(
                    "exactly one key is allowed per action item; found {}",
                    actions
                        .iter()
                        .map(|a| format!("`{a}`"))
                        .collect::<Vec<_>>()
                        .join(" and ")
                ),
            );
            return None;
        }
        let mut allowed: Vec<&str> = ACTION_KEYS.to_vec();
        allowed.push("extensions");
        let before = self.diags.len();
        let f = self.fields(v, path, &allowed, "an action")?;
        let Some(&key) = actions.first() else {
            if self.diags.len() > before {
                return None;
            }
            self.schema(
                v.pos,
                path,
                format!("an action needs exactly one of: {}", ACTION_KEYS.join(", ")),
            );
            return None;
        };
        self.mark(v.pos, path);
        let val = f.get(key)?;
        let p = join(path, key);
        let exts = match f.get("extensions") {
            Some(e) => self.extensions(e, &join(path, "extensions")),
            None => Vec::new(),
        };
        let pos32 = u32::try_from(position).unwrap_or(u32::MAX);
        let kind = match key {
            "raise" => ExecKind::Raise {
                event: self.string(val, &p)?,
            },
            "assign" => {
                let a = self.fields(val, &p, &["location", "expr", "content"], "`assign`")?;
                let Some(location) = self.expr(a.get("location"), &join(&p, "location")) else {
                    self.schema(val.pos, &p, "`assign` needs a `location`");
                    return None;
                };
                ExecKind::Assign {
                    location,
                    expr: self.expr(a.get("expr"), &join(&p, "expr")),
                    content: self.opt_string(a.get("content"), &join(&p, "content")),
                }
            }
            "log" => {
                let a = self.fields(val, &p, &["label", "expr"], "`log`")?;
                ExecKind::Log {
                    label: self.opt_string(a.get("label"), &join(&p, "label")),
                    expr: self.expr(a.get("expr"), &join(&p, "expr")),
                }
            }
            "cancel" => {
                let a = self.fields(val, &p, &["sendid", "sendidexpr"], "`cancel`")?;
                ExecKind::Cancel {
                    sendid: self.opt_string(a.get("sendid"), &join(&p, "sendid")),
                    sendidexpr: self.expr(a.get("sendidexpr"), &join(&p, "sendidexpr")),
                }
            }
            "script" => ExecKind::Script(self.script(val, &p)?),
            "if" => {
                let Some(branches) = val.as_seq() else {
                    self.schema(val.pos, &p, "`if` takes a list of branches `{ cond, do }`");
                    return None;
                };
                let mut out = Vec::new();
                for (i, b) in branches.iter().enumerate() {
                    let bp = index(&p, i);
                    let Some(bf) = self.fields(b, &bp, &["cond", "do"], "an `if` branch") else {
                        continue;
                    };
                    out.push(IfBranch {
                        cond: self.expr(bf.get("cond"), &join(&bp, "cond")),
                        body: bf
                            .get("do")
                            .map(|d| self.block(d, &join(&bp, "do")))
                            .unwrap_or_default(),
                        span: span(b.pos),
                    });
                }
                ExecKind::If { branches: out }
            }
            "foreach" => {
                let a = self.fields(val, &p, &["array", "item", "index", "do"], "`foreach`")?;
                let array = self.expr(a.get("array"), &join(&p, "array"));
                let item = self.opt_string(a.get("item"), &join(&p, "item"));
                let (Some(array), Some(item)) = (array, item) else {
                    self.schema(val.pos, &p, "`foreach` needs `array` and `item`");
                    return None;
                };
                ExecKind::Foreach {
                    array,
                    item,
                    index: self.opt_string(a.get("index"), &join(&p, "index")),
                    body: a
                        .get("do")
                        .map(|d| self.block(d, &join(&p, "do")))
                        .unwrap_or_default(),
                }
            }
            "send" => {
                let (send, ty, ptypes) = self.send(val, &p)?;
                let mut e = Exec {
                    kind: ExecKind::Send(Box::new(send)),
                    extensions: exts,
                    span: span(v.pos),
                };
                if let Some(t) = ty {
                    if vertep_core::types::chart::type_annotation(&e.extensions).is_some() {
                        self.schema(
                            val.pos,
                            &join(&p, "payload_type"),
                            "`payload_type` and a vt:type extension cannot both be given",
                        );
                    }
                    set_send_type(&mut e, Some(&t));
                }
                for (n, t) in ptypes {
                    set_param_type(&mut e.extensions, &n, Some(&t));
                }
                return Some(e);
            }
            "call" => {
                let name = self.string(val, &p)?;
                ExecKind::Extension(Extension {
                    ns: VT_NS.to_string(),
                    local: "call".to_string(),
                    value: ExtensionValue::Element(call_xml(&name)),
                    position: pos32,
                    span: span(val.pos),
                })
            }
            "extension" => {
                let x = self.extension(val, &p, pos32)?;
                ExecKind::Extension(x)
            }
            _ => return None,
        };
        Some(Exec {
            kind,
            extensions: exts,
            span: span(v.pos),
        })
    }

    fn script(&mut self, v: &Node, path: &str) -> Option<Script> {
        if v.as_map().is_some() {
            let f = self.fields(v, path, &["src", "body"], "a script")?;
            return Some(Script {
                src: self.opt_string(f.get("src"), &join(path, "src")),
                body: self.opt_string(f.get("body"), &join(path, "body")),
                span: span(v.pos),
            });
        }
        Some(Script {
            src: None,
            body: Some(self.string(v, path)?),
            span: span(v.pos),
        })
    }

    #[allow(clippy::type_complexity)]
    fn send(&mut self, v: &Node, path: &str) -> Option<(Send, Option<Type>, Vec<(String, Type)>)> {
        let f = self.fields(v, path, SEND_KEYS, "`send`")?;
        let s = |lw: &mut Self, k: &str| lw.opt_string(f.get(k), &join(path, k));
        let x = |lw: &mut Self, k: &str| lw.expr(f.get(k), &join(path, k));
        let mut send = Send {
            event: s(self, "event"),
            eventexpr: x(self, "eventexpr"),
            target: s(self, "target"),
            targetexpr: x(self, "targetexpr"),
            type_: s(self, "type"),
            typeexpr: x(self, "typeexpr"),
            id: s(self, "id"),
            idlocation: x(self, "idlocation"),
            delay: s(self, "delay"),
            delayexpr: x(self, "delayexpr"),
            ..Send::default()
        };
        if let Some(n) = f.get("namelist") {
            send.namelist = self.namelist(n, &join(path, "namelist"));
        }
        let mut ptypes = Vec::new();
        if let Some(p) = f.get("params") {
            send.params = self.params(p, &join(path, "params"), &mut ptypes);
        }
        if let Some(c) = f.get("content") {
            send.content = self.content(c, &join(path, "content"));
        }
        let ty = f
            .get("payload_type")
            .and_then(|t| self.type_spec(t, &join(path, "payload_type")));
        Some((send, ty, ptypes))
    }

    fn namelist(&mut self, v: &Node, path: &str) -> Vec<String> {
        match &v.kind {
            Kind::Seq(_) => self.strings(v, path),
            _ => self
                .string(v, path)
                .map(|s| s.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }

    /// `{ name: expr, ... }` or `[ { name, expr, location, type }, ... ]`; declared types are
    /// returned for the owner's `vt:paramTypes`.
    fn params(&mut self, v: &Node, path: &str, types: &mut Vec<(String, Type)>) -> Vec<Param> {
        let mut out = Vec::new();
        match &v.kind {
            Kind::Map(_) => {
                if let Some(entries) = self.fields_any(v, path, "`params`") {
                    for (name, _, val) in entries {
                        let p = join(path, &name);
                        self.mark(val.pos, &p);
                        out.push(Param {
                            expr: self.expr(Some(val), &p),
                            name,
                            location: None,
                            span: span(val.pos),
                        });
                    }
                }
            }
            Kind::Seq(items) => {
                for (i, n) in items.iter().enumerate() {
                    let p = index(path, i);
                    let Some(f) =
                        self.fields(n, &p, &["name", "expr", "location", "type"], "a param")
                    else {
                        continue;
                    };
                    let Some(name) = f
                        .get("name")
                        .and_then(|x| self.string(x, &join(&p, "name")))
                    else {
                        self.schema(n.pos, &p, "a param needs a `name`");
                        continue;
                    };
                    if let Some(t) = f
                        .get("type")
                        .and_then(|t| self.type_spec(t, &join(&p, "type")))
                    {
                        types.push((name.clone(), t));
                    }
                    self.mark(n.pos, &p);
                    out.push(Param {
                        name,
                        expr: self.expr(f.get("expr"), &join(&p, "expr")),
                        location: self.expr(f.get("location"), &join(&p, "location")),
                        span: span(n.pos),
                    });
                }
            }
            _ if v.is_null() => {}
            _ => self.schema(
                v.pos,
                path,
                "`params` must be a mapping of name to expression or a list",
            ),
        }
        out
    }

    fn content(&mut self, v: &Node, path: &str) -> Option<Content> {
        let f = self.fields(v, path, &["expr", "body"], "`content`")?;
        Some(Content {
            expr: self.expr(f.get("expr"), &join(path, "expr")),
            body: self.opt_string(f.get("body"), &join(path, "body")),
            span: span(v.pos),
        })
    }

    fn donedata(&mut self, v: &Node, path: &str, owner: &mut Vec<Extension>) -> Option<DoneData> {
        let f = self.fields(v, path, &["content", "params"], "`donedata`")?;
        let mut types = Vec::new();
        let params = match f.get("params") {
            Some(p) => self.params(p, &join(path, "params"), &mut types),
            None => Vec::new(),
        };
        for (n, t) in types {
            set_param_type(owner, &n, Some(&t));
        }
        Some(DoneData {
            content: f
                .get("content")
                .and_then(|c| self.content(c, &join(path, "content"))),
            params,
            span: span(v.pos),
        })
    }

    // @tg: implements:: [[openspec:yaml-format#Invoke and Data in YAML]]
    fn invoke(&mut self, v: &Node, path: &str) -> Invoke {
        let mut inv = Invoke {
            span: span(v.pos),
            ..Invoke::default()
        };
        self.mark(v.pos, path);
        let Some(f) = self.fields(v, path, INVOKE_KEYS, "`invoke`") else {
            return inv;
        };
        let s = |lw: &mut Self, k: &str| lw.opt_string(f.get(k), &join(path, k));
        let x = |lw: &mut Self, k: &str| lw.expr(f.get(k), &join(path, k));
        inv.type_ = s(self, "type");
        inv.typeexpr = x(self, "typeexpr");
        inv.src = s(self, "src");
        inv.srcexpr = x(self, "srcexpr");
        inv.id = s(self, "id");
        inv.idlocation = x(self, "idlocation");
        if let Some(n) = f.get("namelist") {
            inv.namelist = self.namelist(n, &join(path, "namelist"));
        }
        if let Some(a) = f.get("autoforward") {
            inv.autoforward = self.bool(a, &join(path, "autoforward"));
        }
        let mut types = Vec::new();
        if let Some(p) = f.get("params") {
            inv.params = self.params(p, &join(path, "params"), &mut types);
        }
        if let Some(c) = f.get("content") {
            inv.content = self.content(c, &join(path, "content"));
        }
        if let Some(b) = f.get("finalize") {
            inv.finalize = self.block(b, &join(path, "finalize"));
        }
        if let Some(e) = f.get("extensions") {
            inv.extensions = self.extensions(e, &join(path, "extensions"));
        }
        for (n, t) in types {
            set_param_type(&mut inv.extensions, &n, Some(&t));
        }
        inv
    }

    // ============================================================================ data

    // @tg: implements:: [[openspec:yaml-format#Invoke and Data in YAML]]
    fn data_specs(&mut self, v: &Node, path: &str) -> Vec<Data> {
        let mut out = Vec::new();
        match &v.kind {
            Kind::Map(_) => {
                if let Some(entries) = self.fields_any(v, path, "`context`") {
                    for (id, k, val) in entries {
                        let p = join(path, &id);
                        if let Some(d) = self.data(id, k.pos, val, &p, DATA_KEYS) {
                            out.push(d);
                        }
                    }
                }
            }
            Kind::Seq(items) => {
                for (i, n) in items.iter().enumerate() {
                    let p = index(path, i);
                    let id = n
                        .as_map()
                        .and_then(|m| m.iter().find(|(k, _)| k.scalar() == Some("id")))
                        .and_then(|(_, v)| v.scalar().map(str::to_string));
                    let Some(id) = id else {
                        self.schema(n.pos, &p, "a context entry in list form needs an `id`");
                        continue;
                    };
                    if let Some(d) = self.data(id, n.pos, n, &p, DATA_LIST_KEYS) {
                        out.push(d);
                    }
                }
            }
            _ if v.is_null() => {}
            _ => self.schema(
                v.pos,
                path,
                "`context` must be a mapping of field to declaration",
            ),
        }
        out
    }

    fn data(&mut self, id: String, at: Pos, v: &Node, path: &str, keys: &[&str]) -> Option<Data> {
        self.mark(at, path);
        let mut d = Data {
            id,
            src: None,
            expr: None,
            content: None,
            extensions: Vec::new(),
            span: span(at),
        };
        if v.is_null() {
            return Some(d);
        }
        let f = self.fields(v, path, keys, "a context entry")?;
        d.expr = self.expr(f.get("init"), &join(path, "init"));
        d.src = self.opt_string(f.get("src"), &join(path, "src"));
        d.content = self.opt_string(f.get("content"), &join(path, "content"));
        if let Some(e) = f.get("extensions") {
            d.extensions = self.extensions(e, &join(path, "extensions"));
        }
        if let Some(t) = f
            .get("type")
            .and_then(|t| self.type_spec(t, &join(path, "type")))
        {
            if vertep_core::types::chart::type_annotation(&d.extensions).is_some() {
                self.schema(
                    v.pos,
                    &join(path, "type"),
                    "`type` and a vt:type extension cannot both be given",
                );
            }
            set_data_type(&mut d, Some(&t));
        }
        Some(d)
    }

    // ============================================================================ extensions

    fn extensions(&mut self, v: &Node, path: &str) -> Vec<Extension> {
        let Some(items) = v.as_seq() else {
            self.schema(v.pos, path, "`extensions` must be a list");
            return Vec::new();
        };
        items
            .iter()
            .enumerate()
            .filter_map(|(i, n)| self.extension(n, &index(path, i), 0))
            .collect()
    }

    fn extension(&mut self, v: &Node, path: &str, default_position: u32) -> Option<Extension> {
        let f = self.fields(v, path, EXTENSION_KEYS, "an extension")?;
        let ns = self
            .opt_string(f.get("ns"), &join(path, "ns"))
            .unwrap_or_else(|| VT_NS.to_string());
        let Some(local) = f
            .get("local")
            .and_then(|l| self.string(l, &join(path, "local")))
        else {
            self.schema(v.pos, path, "an extension needs `local`");
            return None;
        };
        let value = match (f.get("attribute"), f.get("element")) {
            (Some(a), None) => ExtensionValue::Attribute(
                a.scalar()
                    .filter(|_| !a.is_null())
                    .unwrap_or_default()
                    .to_string(),
            ),
            (None, Some(e)) => ExtensionValue::Element(self.string(e, &join(path, "element"))?),
            _ => {
                self.schema(
                    v.pos,
                    path,
                    "an extension needs exactly one of `attribute` and `element`",
                );
                return None;
            }
        };
        let position = match f.get("position") {
            Some(p) => match p.as_u32() {
                Some(n) => n,
                None => {
                    self.schema(
                        p.pos,
                        &join(path, "position"),
                        "`position` must be a non-negative integer",
                    );
                    0
                }
            },
            None => default_position,
        };
        Some(Extension {
            ns,
            local,
            value,
            position,
            span: span(v.pos),
        })
    }

    // ============================================================================ checks

    fn path_of(&self, s: Span) -> String {
        self.paths
            .get(&Pos {
                line: s.line,
                column: s.column,
            })
            .cloned()
            .unwrap_or_default()
    }

    /// Duplicate state ids and unknown targets, located.
    fn check_references(&mut self) {
        let ids = std::mem::take(&mut self.ids);
        for (i, (id, pos, path)) in ids.iter().enumerate() {
            let others: Vec<&(String, Pos, String)> = ids
                .iter()
                .enumerate()
                .filter(|(j, (o, _, _))| *j != i && o == id)
                .map(|(_, x)| x)
                .collect();
            if !others.is_empty() {
                let lines: Vec<String> =
                    others.iter().map(|(_, p, _)| p.line.to_string()).collect();
                self.err(
                    ErrorKind::Reference,
                    *pos,
                    path,
                    format!(
                        "duplicate state id `{id}` (also at line {})",
                        lines.join(", ")
                    ),
                );
            }
        }
        let targets = std::mem::take(&mut self.targets);
        for (t, pos, path) in &targets {
            if !ids.iter().any(|(id, _, _)| id == t) {
                let mut msg = format!("unknown target `{t}`");
                if let Some(s) = suggest(t, ids.iter().map(|(id, _, _)| id.as_str())) {
                    msg.push_str(&format!("; did you mean `{s}`?"));
                }
                self.err(ErrorKind::Reference, *pos, path, msg);
            }
        }
        self.ids = ids;
        self.targets = targets;
    }

    fn validate(&mut self, chart: &Chart) {
        if let Err(violations) = validate(chart) {
            for v in violations {
                if matches!(
                    v.kind,
                    ViolationKind::DuplicateStateId { .. } | ViolationKind::UnknownTarget { .. }
                ) {
                    continue;
                }
                let path = self.path_of(v.span);
                self.err(
                    ErrorKind::Validation,
                    Pos {
                        line: v.span.line,
                        column: v.span.column,
                    },
                    &path,
                    v.to_string(),
                );
            }
        }
    }

    // @tg: implements:: [[openspec:yaml-format#YAML Uses Expression Strings of Its Datamodel]] {confidence: partial, note: "type checks with the core BasicTyper unless a datamodel typer is supplied"}
    fn typecheck(&mut self, chart: &Chart) {
        if let Some(d) = self.decls.clone() {
            for e in d.validate() {
                let pos = Pos {
                    line: e.span.line,
                    column: e.span.column,
                };
                let path = self
                    .decl_paths
                    .iter()
                    .find(|(p, _)| *p == pos)
                    .map(|(_, p)| p.clone())
                    .unwrap_or_else(|| "types".to_string());
                self.err(ErrorKind::Type, pos, &path, e.to_string());
            }
        }
        let fallback = BasicTyper::for_datamodel(&chart.datamodel);
        let typer = self.options.typer.unwrap_or(&fallback);
        if let Err(diags) = check_chart(chart, typer) {
            for d in diags {
                if d.kind == DiagnosticKind::Declaration && self.decls.is_some() {
                    continue;
                }
                let path = self.path_of(d.span);
                self.err(
                    ErrorKind::Type,
                    Pos {
                        line: d.span.line,
                        column: d.span.column,
                    },
                    &path,
                    format!("{}: {}", d.location, d.message),
                );
            }
        }
    }
}

fn versions() -> String {
    SUPPORTED_VERSIONS
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn read(input: &str, options: &ReadOptions<'_>) -> Result<Chart, LoadError> {
    let file = options.file.map(str::to_string);
    let root = parse(input.strip_prefix('\u{feff}').unwrap_or(input)).map_err(|e| LoadError {
        diagnostics: vec![Diagnostic {
            kind: ErrorKind::Syntax,
            file: file.clone(),
            line: e.pos.line,
            column: e.pos.column,
            path: String::new(),
            message: format!("YAML syntax error: {}", e.message),
        }],
    })?;
    let mut lw = Lower::new(*options);
    let chart = lw.chart(&root);
    if let Some(chart) = &chart {
        if options.validate {
            lw.check_references();
            if lw.diags.is_empty() {
                lw.validate(chart);
            }
        }
        if options.typecheck && lw.diags.is_empty() {
            lw.typecheck(chart);
        }
    }
    let mut diagnostics = lw.diags;
    match chart {
        Some(chart) if diagnostics.is_empty() => Ok(chart),
        _ => {
            diagnostics.sort_by_key(|d| (d.line, d.column));
            Err(LoadError { diagnostics })
        }
    }
}
