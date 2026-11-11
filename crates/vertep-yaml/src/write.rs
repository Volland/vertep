//! The canonical YAML writer: chart IR to a node tree, then block-style text.
//!
//! Every shorthand (`on` grouping, `types` blocks, `type` on context entries, `payload_type`,
//! param types, `call`) is used only when re-lowering it gives back exactly the IR node;
//! otherwise the explicit form is written. The finished document is read back and compared
//! with the chart before it is returned.

use std::collections::HashSet;

use vertep_core::ir::{
    Binding, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, Invoke, Param, Script, Send, State, StateElement, Transition, TransitionType,
    VT_NS,
};
use vertep_core::types::chart::{
    PARAM_TYPES_ATTR, TYPE_ATTR, declarations, declarations_element, set_data_type,
    set_declarations, set_param_type, set_send_type, type_annotation,
};
use vertep_core::types::{Type, parse_type};

use crate::node::{Node, emit};
use crate::read::call_xml;
use crate::types_yaml::type_to_node;
use crate::{DEFAULT_DATAMODEL, ReadOptions, WriteError};

pub(crate) fn write(chart: &Chart) -> Result<String, WriteError> {
    let mut seen = HashSet::new();
    let mut dup = None;
    walk_ids(&chart.states, &mut |id| {
        if !seen.insert(id.to_string()) && dup.is_none() {
            dup = Some(id.to_string());
        }
    });
    if let Some(id) = dup {
        return Err(WriteError::DuplicateStateId(id));
    }
    let text = emit(&chart_node(chart));
    match crate::read::read(&text, &ReadOptions::lower_only()) {
        Ok(back) if back == *chart => Ok(text),
        _ => Err(WriteError::NotLossless),
    }
}

fn walk_ids(states: &[State], f: &mut dyn FnMut(&str)) {
    for s in states {
        f(&s.id);
        walk_ids(&s.children, f);
    }
}

fn opt(entries: &mut Vec<(&'static str, Node)>, key: &'static str, v: &Option<String>) {
    if let Some(s) = v {
        entries.push((key, Node::str(s.clone())));
    }
}

fn refs(targets: &[String]) -> Node {
    match targets {
        [one] => Node::str(one.clone()),
        many => Node::seq(many.iter().map(|t| Node::str(t.clone())).collect()),
    }
}

fn flagged(value: &str, explicit: bool) -> Node {
    if explicit {
        Node::str(value)
    } else {
        Node::map(vec![
            ("value", Node::str(value)),
            ("implicit", Node::bool(true)),
        ])
    }
}

fn is_attr(e: &Extension, local: &str) -> bool {
    e.ns == VT_NS && e.local == local && matches!(e.value, ExtensionValue::Attribute(_))
}

fn without(exts: &[Extension], local: &str) -> Vec<Extension> {
    exts.iter()
        .filter(|e| !is_attr(e, local))
        .cloned()
        .collect()
}

// ================================================================================ chart

fn chart_node(c: &Chart) -> Node {
    let mut e: Vec<(&'static str, Node)> = vec![("vertep", Node::raw("1"))];
    e.push((
        "name",
        c.name
            .as_ref()
            .map_or_else(Node::null, |n| Node::str(n.clone())),
    ));
    if c.datamodel_explicit || c.datamodel != DEFAULT_DATAMODEL {
        e.push(("datamodel", flagged(&c.datamodel, c.datamodel_explicit)));
    }
    if c.binding_explicit || c.binding == Binding::Late {
        let b = if c.binding == Binding::Late {
            "late"
        } else {
            "early"
        };
        e.push(("binding", flagged(b, c.binding_explicit)));
    }
    let mut rest = c.extensions.clone();
    if let (Some(el), Some(Ok(d))) = (declarations_element(c), declarations(c)) {
        let mut test = c.clone();
        test.extensions.retain(|x| x != el);
        let remaining = test.extensions.clone();
        set_declarations(&mut test, Some(&d));
        let unique_types: HashSet<&str> = d.types.iter().map(|t| t.name.as_str()).collect();
        let unique_events: HashSet<&str> = d.events.iter().map(|t| t.name.as_str()).collect();
        if test.extensions == c.extensions
            && unique_types.len() == d.types.len()
            && unique_events.len() == d.events.len()
        {
            rest = remaining;
            if !d.types.is_empty() || d.events.is_empty() {
                e.push((
                    "types",
                    Node::map_owned(
                        d.types
                            .iter()
                            .map(|t| (t.name.clone(), type_to_node(&t.ty)))
                            .collect(),
                    ),
                ));
            }
            if !d.events.is_empty() {
                e.push((
                    "events",
                    Node::map_owned(
                        d.events
                            .iter()
                            .map(|ev| {
                                let p = ev.payload.as_ref().map_or_else(Node::null, type_to_node);
                                (ev.name.clone(), p)
                            })
                            .collect(),
                    ),
                ));
            }
        }
    }
    if !c.data.is_empty() {
        e.push(("context", data_specs(&c.data)));
    }
    if let Some(s) = &c.script {
        e.push(("script", script(s)));
    }
    if !c.initial.is_empty() {
        e.push(("initial", refs(&c.initial)));
    }
    e.push(("states", states(&c.states)));
    if !rest.is_empty() {
        e.push(("extensions", extensions(&rest, 0)));
    }
    Node::map(e)
}

fn states(list: &[State]) -> Node {
    Node::map_owned(list.iter().map(|s| (s.id.clone(), state(s))).collect())
}

fn state(s: &State) -> Node {
    let mut e: Vec<(&'static str, Node)> = Vec::new();
    match s.element {
        StateElement::State => {}
        StateElement::Parallel => e.push(("type", Node::str("parallel"))),
        StateElement::Final => e.push(("type", Node::str("final"))),
        StateElement::History(h) => {
            e.push(("type", Node::str("history")));
            if h == HistoryType::Deep {
                e.push(("history", Node::str("deep")));
            }
        }
    }
    if s.id_generated {
        e.push(("generated", Node::bool(true)));
    }
    if !s.initial.is_empty() {
        e.push(("initial", refs(&s.initial)));
    }
    if let Some(i) = &s.initial_element {
        let mut m = vec![("transition", transition_map(&i.transition, true))];
        if !i.extensions.is_empty() {
            m.push(("extensions", extensions(&i.extensions, 0)));
        }
        e.push(("initial_element", Node::map(m)));
    }
    if !s.data.is_empty() {
        e.push(("context", data_specs(&s.data)));
    }
    if !s.onentry.is_empty() {
        e.push(("entry", blocks(&s.onentry)));
    }
    if !s.onexit.is_empty() {
        e.push(("exit", blocks(&s.onexit)));
    }
    match s.invokes.as_slice() {
        [] => {}
        [one] => e.push(("invoke", invoke(one))),
        many => e.push(("invoke", Node::seq(many.iter().map(invoke).collect()))),
    }
    e.extend(transitions(&s.transitions));
    let mut exts = s.extensions.clone();
    if let Some(dd) = &s.donedata {
        let (node, rest) = donedata(dd, &s.extensions);
        e.push(("donedata", node));
        exts = rest;
    }
    if !s.children.is_empty() {
        e.push(("states", states(&s.children)));
    }
    if !exts.is_empty() {
        e.push(("extensions", extensions(&exts, 0)));
    }
    Node::map(e)
}

// ================================================================================ transitions

fn simple(t: &Transition) -> bool {
    t.cond.is_none()
        && !t.kind_explicit
        && t.kind == TransitionType::External
        && t.body.is_empty()
        && t.extensions.is_empty()
}

fn compact(t: &Transition) -> Node {
    if simple(t) {
        match t.targets.as_slice() {
            [] => Node::map(vec![]),
            targets => refs(targets),
        }
    } else {
        transition_map(t, false)
    }
}

fn plain_events(events: &[String]) -> bool {
    events
        .iter()
        .all(|e| !e.is_empty() && !e.contains(char::is_whitespace))
}

fn transition_map(t: &Transition, with_event: bool) -> Node {
    let mut e: Vec<(&'static str, Node)> = Vec::new();
    if with_event && !t.events.is_empty() {
        let ev = if plain_events(&t.events) {
            Node::str(t.events.join(" "))
        } else {
            Node::seq(t.events.iter().map(|x| Node::str(x.clone())).collect())
        };
        e.push(("event", ev));
    }
    opt(&mut e, "cond", &t.cond);
    if !t.targets.is_empty() {
        e.push(("target", refs(&t.targets)));
    }
    if t.kind_explicit || t.kind == TransitionType::Internal {
        let k = if t.kind == TransitionType::Internal {
            "internal"
        } else {
            "external"
        };
        e.push(("type", flagged(k, t.kind_explicit)));
    }
    if !t.body.is_empty() {
        e.push(("actions", block(&t.body)));
    }
    if !t.extensions.is_empty() {
        e.push(("extensions", extensions(&t.extensions, 0)));
    }
    Node::map(e)
}

fn group(ts: &[&Transition]) -> Node {
    match ts {
        [one] => compact(one),
        many => Node::seq(many.iter().map(|t| transition_map(t, false)).collect()),
    }
}

/// `on` and `always` when grouping keeps the IR order, otherwise an explicit `transitions` list.
fn transitions(ts: &[Transition]) -> Vec<(&'static str, Node)> {
    if ts.is_empty() {
        return Vec::new();
    }
    let eventful: Vec<usize> = (0..ts.len())
        .filter(|&i| !ts[i].events.is_empty())
        .collect();
    let eventless: Vec<usize> = (0..ts.len()).filter(|&i| ts[i].events.is_empty()).collect();
    let groupable = eventful.iter().all(|&i| plain_events(&ts[i].events));
    if groupable {
        let mut keys: Vec<String> = Vec::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for &i in &eventful {
            let k = ts[i].events.join(" ");
            match keys.iter().position(|x| *x == k) {
                Some(g) => groups[g].push(i),
                None => {
                    keys.push(k);
                    groups.push(vec![i]);
                }
            }
        }
        let on_order: Vec<usize> = groups.iter().flatten().copied().collect();
        let on_node = || {
            Node::map_owned(
                keys.iter()
                    .zip(&groups)
                    .map(|(k, g)| {
                        let refs: Vec<&Transition> = g.iter().map(|&i| &ts[i]).collect();
                        (k.clone(), group(&refs))
                    })
                    .collect(),
            )
        };
        let always_node = || {
            let refs: Vec<&Transition> = eventless.iter().map(|&i| &ts[i]).collect();
            group(&refs)
        };
        let original: Vec<usize> = (0..ts.len()).collect();
        let on_first: Vec<usize> = on_order.iter().chain(&eventless).copied().collect();
        let always_first: Vec<usize> = eventless.iter().chain(&on_order).copied().collect();
        let mut out = Vec::new();
        if on_first == original {
            if !eventful.is_empty() {
                out.push(("on", on_node()));
            }
            if !eventless.is_empty() {
                out.push(("always", always_node()));
            }
            return out;
        }
        if always_first == original {
            out.push(("always", always_node()));
            out.push(("on", on_node()));
            return out;
        }
    }
    vec![(
        "transitions",
        Node::seq(ts.iter().map(|t| transition_map(t, true)).collect()),
    )]
}

// ================================================================================ content

fn blocks(bs: &[Vec<Exec>]) -> Node {
    match bs {
        [one] => block(one),
        many => Node::seq(many.iter().map(|b| block(b)).collect()),
    }
}

fn block(b: &[Exec]) -> Node {
    Node::seq(b.iter().enumerate().map(|(i, e)| item(e, i)).collect())
}

/// The function name of a canonical `vt:call` element.
fn call_name(x: &Extension) -> Option<String> {
    let ExtensionValue::Element(xml) = &x.value else {
        return None;
    };
    if x.ns != VT_NS || x.local != "call" {
        return None;
    }
    let prefix = format!("<vt:call xmlns:vt=\"{VT_NS}\" name=\"");
    let raw = xml.strip_prefix(&prefix)?.strip_suffix("\"/>")?;
    let name = raw
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#9;", "\t")
        .replace("&#10;", "\n")
        .replace("&#13;", "\r")
        .replace("&amp;", "&");
    (call_xml(&name) == *xml).then_some(name)
}

fn item(x: &Exec, position: usize) -> Node {
    let mut exts = x.extensions.clone();
    let (key, val): (&'static str, Node) = match &x.kind {
        ExecKind::Raise { event } => ("raise", Node::str(event.clone())),
        ExecKind::Assign {
            location,
            expr,
            content,
        } => {
            let mut m = vec![("location", Node::str(location.clone()))];
            opt(&mut m, "expr", expr);
            opt(&mut m, "content", content);
            ("assign", Node::map(m))
        }
        ExecKind::Log { label, expr } => {
            let mut m = Vec::new();
            opt(&mut m, "label", label);
            opt(&mut m, "expr", expr);
            ("log", Node::map(m))
        }
        ExecKind::Cancel { sendid, sendidexpr } => {
            let mut m = Vec::new();
            opt(&mut m, "sendid", sendid);
            opt(&mut m, "sendidexpr", sendidexpr);
            ("cancel", Node::map(m))
        }
        ExecKind::Script(s) => ("script", script(s)),
        ExecKind::If { branches } => (
            "if",
            Node::seq(
                branches
                    .iter()
                    .map(|b| {
                        let mut m = Vec::new();
                        opt(&mut m, "cond", &b.cond);
                        if !b.body.is_empty() {
                            m.push(("do", block(&b.body)));
                        }
                        Node::map(m)
                    })
                    .collect(),
            ),
        ),
        ExecKind::Foreach {
            array,
            item,
            index,
            body,
        } => {
            let mut m = vec![
                ("array", Node::str(array.clone())),
                ("item", Node::str(item.clone())),
            ];
            opt(&mut m, "index", index);
            if !body.is_empty() {
                m.push(("do", block(body)));
            }
            ("foreach", Node::map(m))
        }
        ExecKind::Send(s) => {
            let (node, rest) = send(s, &x.extensions);
            exts = rest;
            ("send", node)
        }
        ExecKind::Extension(e) => match call_name(e) {
            Some(name) if e.position as usize == position => ("call", Node::str(name)),
            _ => ("extension", extension(e, position as u32)),
        },
    };
    let mut m = vec![(key, val)];
    if !exts.is_empty() {
        m.push(("extensions", extensions(&exts, 0)));
    }
    Node::map(m)
}

fn script(s: &Script) -> Node {
    match (&s.src, &s.body) {
        (None, Some(b)) => Node::str(b.clone()),
        _ => {
            let mut m = Vec::new();
            opt(&mut m, "src", &s.src);
            opt(&mut m, "body", &s.body);
            Node::map(m)
        }
    }
}

fn content(c: &Content) -> Node {
    let mut m = Vec::new();
    opt(&mut m, "expr", &c.expr);
    opt(&mut m, "body", &c.body);
    Node::map(m)
}

/// Param type shorthand: the declared types of `params` and the owner's other extensions,
/// when re-applying the types in param order rebuilds `owner` exactly.
fn param_sugar(
    params: &[Param],
    owner: &[Extension],
) -> Option<(Vec<Option<Type>>, Vec<Extension>)> {
    let attr = owner.iter().find(|e| is_attr(e, PARAM_TYPES_ATTR))?;
    let ExtensionValue::Attribute(text) = &attr.value else {
        return None;
    };
    let Ok(Type::Record(fields)) = parse_type(text) else {
        return None;
    };
    let types: Vec<Option<Type>> = params
        .iter()
        .map(|p| {
            fields
                .iter()
                .find(|f| f.name == p.name)
                .map(|f| f.ty.clone())
        })
        .collect();
    let rest = without(owner, PARAM_TYPES_ATTR);
    let mut rebuilt = rest.clone();
    for (p, t) in params.iter().zip(&types) {
        if let Some(t) = t {
            set_param_type(&mut rebuilt, &p.name, Some(t));
        }
    }
    (rebuilt == owner).then_some((types, rest))
}

fn params(ps: &[Param], types: &[Option<Type>]) -> Node {
    let mut names = HashSet::new();
    let short = types.iter().all(Option::is_none)
        && ps
            .iter()
            .all(|p| p.location.is_none() && names.insert(p.name.as_str()));
    if short {
        return Node::map_owned(
            ps.iter()
                .map(|p| {
                    let v = p
                        .expr
                        .as_ref()
                        .map_or_else(Node::null, |x| Node::str(x.clone()));
                    (p.name.clone(), v)
                })
                .collect(),
        );
    }
    Node::seq(
        ps.iter()
            .enumerate()
            .map(|(i, p)| {
                let mut m = vec![("name", Node::str(p.name.clone()))];
                opt(&mut m, "expr", &p.expr);
                opt(&mut m, "location", &p.location);
                if let Some(Some(t)) = types.get(i) {
                    m.push(("type", type_to_node(t)));
                }
                Node::map(m)
            })
            .collect(),
    )
}

fn send(s: &Send, exec_exts: &[Extension]) -> (Node, Vec<Extension>) {
    // Shorthands: `payload_type` (vt:type) and param types (vt:paramTypes).
    let payload = type_annotation(exec_exts).and_then(|t| parse_type(t).ok());
    let mut rest = exec_exts.to_vec();
    let mut payload_sugar = None;
    let mut ptypes: Vec<Option<Type>> = vec![None; s.params.len()];
    let base = without(exec_exts, TYPE_ATTR);
    let (types, base2) =
        param_sugar(&s.params, &base).unwrap_or_else(|| (vec![None; s.params.len()], base.clone()));
    let mut rebuilt = Exec::new(ExecKind::Raise {
        event: String::new(),
    });
    rebuilt.extensions = base2.clone();
    if let Some(t) = &payload {
        set_send_type(&mut rebuilt, Some(t));
    }
    for (p, t) in s.params.iter().zip(&types) {
        if let Some(t) = t {
            set_param_type(&mut rebuilt.extensions, &p.name, Some(t));
        }
    }
    if rebuilt.extensions == exec_exts && (payload.is_some() || types.iter().any(Option::is_some)) {
        payload_sugar = payload;
        ptypes = types;
        rest = base2;
    }
    let mut m: Vec<(&'static str, Node)> = Vec::new();
    opt(&mut m, "event", &s.event);
    opt(&mut m, "eventexpr", &s.eventexpr);
    opt(&mut m, "target", &s.target);
    opt(&mut m, "targetexpr", &s.targetexpr);
    opt(&mut m, "type", &s.type_);
    opt(&mut m, "typeexpr", &s.typeexpr);
    opt(&mut m, "id", &s.id);
    opt(&mut m, "idlocation", &s.idlocation);
    opt(&mut m, "delay", &s.delay);
    opt(&mut m, "delayexpr", &s.delayexpr);
    if !s.namelist.is_empty() {
        m.push((
            "namelist",
            Node::seq(s.namelist.iter().map(|n| Node::str(n.clone())).collect()),
        ));
    }
    if !s.params.is_empty() {
        m.push(("params", params(&s.params, &ptypes)));
    }
    if let Some(c) = &s.content {
        m.push(("content", content(c)));
    }
    if let Some(t) = &payload_sugar {
        m.push(("payload_type", type_to_node(t)));
    }
    (Node::map(m), rest)
}

fn invoke(inv: &Invoke) -> Node {
    let (types, exts) = param_sugar(&inv.params, &inv.extensions)
        .unwrap_or_else(|| (vec![None; inv.params.len()], inv.extensions.clone()));
    let mut m: Vec<(&'static str, Node)> = Vec::new();
    opt(&mut m, "id", &inv.id);
    opt(&mut m, "idlocation", &inv.idlocation);
    opt(&mut m, "type", &inv.type_);
    opt(&mut m, "typeexpr", &inv.typeexpr);
    opt(&mut m, "src", &inv.src);
    opt(&mut m, "srcexpr", &inv.srcexpr);
    if !inv.namelist.is_empty() {
        m.push((
            "namelist",
            Node::seq(inv.namelist.iter().map(|n| Node::str(n.clone())).collect()),
        ));
    }
    if inv.autoforward {
        m.push(("autoforward", Node::bool(true)));
    }
    if !inv.params.is_empty() {
        m.push(("params", params(&inv.params, &types)));
    }
    if let Some(c) = &inv.content {
        m.push(("content", content(c)));
    }
    if !inv.finalize.is_empty() {
        m.push(("finalize", block(&inv.finalize)));
    }
    if !exts.is_empty() {
        m.push(("extensions", extensions(&exts, 0)));
    }
    Node::map(m)
}

fn donedata(dd: &DoneData, state_exts: &[Extension]) -> (Node, Vec<Extension>) {
    let (types, rest) = param_sugar(&dd.params, state_exts)
        .unwrap_or_else(|| (vec![None; dd.params.len()], state_exts.to_vec()));
    let mut m: Vec<(&'static str, Node)> = Vec::new();
    if let Some(c) = &dd.content {
        m.push(("content", content(c)));
    }
    if !dd.params.is_empty() {
        m.push(("params", params(&dd.params, &types)));
    }
    (Node::map(m), rest)
}

// ================================================================================ data

fn data_specs(ds: &[Data]) -> Node {
    let mut ids = HashSet::new();
    let unique = ds.iter().all(|d| ids.insert(d.id.as_str()));
    if unique {
        Node::map_owned(ds.iter().map(|d| (d.id.clone(), data(d, false))).collect())
    } else {
        Node::seq(ds.iter().map(|d| data(d, true)).collect())
    }
}

fn data(d: &Data, with_id: bool) -> Node {
    let mut exts = d.extensions.clone();
    let mut ty = None;
    if let Some(t) = type_annotation(&d.extensions).and_then(|t| parse_type(t).ok()) {
        let mut test = d.clone();
        test.extensions = without(&d.extensions, TYPE_ATTR);
        let rest = test.extensions.clone();
        set_data_type(&mut test, Some(&t));
        if test.extensions == d.extensions {
            ty = Some(t);
            exts = rest;
        }
    }
    let mut m: Vec<(&'static str, Node)> = Vec::new();
    if with_id {
        m.push(("id", Node::str(d.id.clone())));
    }
    if let Some(t) = &ty {
        m.push(("type", type_to_node(t)));
    }
    opt(&mut m, "init", &d.expr);
    opt(&mut m, "src", &d.src);
    opt(&mut m, "content", &d.content);
    if !exts.is_empty() {
        m.push(("extensions", extensions(&exts, 0)));
    }
    Node::map(m)
}

// ================================================================================ extensions

fn extensions(exts: &[Extension], default_position: u32) -> Node {
    Node::seq(
        exts.iter()
            .map(|e| extension(e, default_position))
            .collect(),
    )
}

fn extension(e: &Extension, default_position: u32) -> Node {
    let mut m: Vec<(&'static str, Node)> = Vec::new();
    if e.ns != VT_NS {
        m.push(("ns", Node::str(e.ns.clone())));
    }
    m.push(("local", Node::str(e.local.clone())));
    match &e.value {
        ExtensionValue::Attribute(v) => m.push(("attribute", Node::str(v.clone()))),
        ExtensionValue::Element(x) => m.push(("element", Node::str(x.clone()))),
    }
    if e.position != default_position {
        m.push(("position", Node::raw(e.position.to_string())));
    }
    Node::map(m)
}
