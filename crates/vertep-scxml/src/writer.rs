//! The SCXML writer.

use core::fmt;
use std::collections::BTreeSet;

use quick_xml::Reader;
use quick_xml::events::Event;
use vertep_core::ir::{
    Binding, Chart, Content, Data, DoneData, Exec, ExecKind, Extension, ExtensionValue,
    HistoryType, IfBranch, Initial, Invoke, Param, SCXML_NS, Script, State, StateElement,
    Transition, TransitionType, VT_NS,
};

use std::borrow::Cow;

use vertep_core::types::Type;
use vertep_core::types::chart::{PARAM_TYPES_ATTR, param_types};

use crate::ids::referenced_ids;
use crate::text::{escape_attr, escape_text, is_ncname, is_xml_char, looks_like_xml};

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

/// Why a chart could not be written. Every problem found is listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteError {
    /// One line per problem, naming the node involved.
    pub problems: Vec<String>,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "chart cannot be written as SCXML:")?;
        for p in &self.problems {
            write!(f, "\n  {p}")?;
        }
        Ok(())
    }
}

impl std::error::Error for WriteError {}

/// Write a chart as an SCXML 1.0 document ([[openspec:scxml-format#SCXML Serialization]]).
///
/// The output is UTF-8 with an XML declaration, the SCXML namespace as default namespace,
/// `version="1.0"` and two-space indentation. Generated state identifiers are omitted unless a
/// transition target or `initial` attribute names the state. `vt:` extension attributes are
/// written with the `vt` prefix and foreign ones with `ns1`, `ns2`, … declared on the root;
/// extension elements are written verbatim (they carry their own namespace declarations) at
/// their recorded position among their owner's children.
///
/// Children are written in a fixed order per element (for a state: `initial`, `datamodel`,
/// `onentry`, `onexit`, `transition`, child states, `invoke`, `donedata`), so the writer is a
/// function of the IR alone. A `src` that the reader resolved is written as the `src`
/// attribute only.
///
/// Fails, listing every problem, when the chart holds something XML cannot carry: characters
/// outside XML 1.0, extension names that are not `NCName`s or that are in the SCXML namespace,
/// extension elements that are not one well-formed element, extension elements on elements
/// that cannot have children, or an `if` whose first branch has no condition or whose `else`
/// is not last.
// @tg: implements:: [[openspec:scxml-format#SCXML Serialization]]
pub fn write(chart: &Chart) -> Result<String, WriteError> {
    let mut w = Writer {
        out: String::new(),
        indent: 0,
        prefixes: Vec::new(),
        referenced: referenced_ids(chart),
        problems: Vec::new(),
    };
    w.collect_prefixes(chart);
    w.chart(chart);
    if w.problems.is_empty() {
        Ok(w.out)
    } else {
        Err(WriteError {
            problems: w.problems,
        })
    }
}

/// A child of an element, in output order.
enum Item<'c> {
    State(&'c State),
    Datamodel(&'c [Data]),
    Script(&'c Script),
    Onentry(&'c [Exec]),
    Onexit(&'c [Exec]),
    Transition(&'c Transition),
    Initial(&'c Initial),
    Invoke(&'c Invoke),
    /// A `<donedata>` and the `vt:type` of each of its params.
    DoneData(&'c DoneData, Vec<Option<String>>),
    /// A `<param>` and its `vt:type`.
    Param(&'c Param, Option<String>),
    Content(&'c Content),
    Finalize(&'c [Exec]),
    Ext(&'c Extension),
}

struct Writer {
    out: String,
    indent: usize,
    /// Namespace IRI and prefix of every extension attribute namespace.
    prefixes: Vec<(String, String)>,
    referenced: BTreeSet<String>,
    problems: Vec<String>,
}

impl Writer {
    // ---------------------------------------------------------------- namespaces

    fn collect_prefixes(&mut self, chart: &Chart) {
        let mut namespaces: Vec<String> = Vec::new();
        let mut add = |exts: &[Extension]| {
            for x in exts {
                if matches!(x.value, ExtensionValue::Attribute(_)) && !namespaces.contains(&x.ns) {
                    namespaces.push(x.ns.clone());
                }
            }
        };
        visit_extensions(chart, &mut add);
        let mut n = 0;
        for ns in namespaces {
            let prefix = if ns == VT_NS {
                "vt".to_owned()
            } else if ns == XML_NS {
                "xml".to_owned()
            } else {
                n += 1;
                format!("ns{n}")
            };
            self.prefixes.push((ns, prefix));
        }
    }

    fn prefix_of(&self, ns: &str) -> Option<&str> {
        self.prefixes
            .iter()
            .find(|(n, _)| n == ns)
            .map(|(_, p)| p.as_str())
    }

    // ---------------------------------------------------------------- output primitives

    fn check_chars(&mut self, s: &str, what: &str) -> bool {
        match s.chars().find(|c| !is_xml_char(*c)) {
            Some(c) => {
                self.problems.push(format!(
                    "{what} contains U+{:04X}, which XML 1.0 cannot carry",
                    u32::from(c)
                ));
                false
            }
            None => true,
        }
    }

    fn line_start(&mut self) {
        for _ in 0..self.indent {
            self.out.push_str("  ");
        }
    }

    /// Write `<name a="v" …` plus extension attributes; the caller closes the tag.
    fn open(&mut self, name: &str, attrs: &[(&str, Option<&str>)], exts: &[Extension]) {
        self.line_start();
        self.out.push('<');
        self.out.push_str(name);
        for (k, v) in attrs {
            if let Some(v) = v {
                self.attr(k, v, name);
            }
        }
        for x in exts {
            if let ExtensionValue::Attribute(v) = &x.value {
                self.ext_attr(x, v, name);
            }
        }
    }

    fn attr(&mut self, key: &str, value: &str, owner: &str) {
        if !self.check_chars(value, &format!("attribute `{key}` of <{owner}>")) {
            return;
        }
        self.out.push(' ');
        self.out.push_str(key);
        self.out.push_str("=\"");
        escape_attr(value, &mut self.out);
        self.out.push('"');
    }

    fn ext_attr(&mut self, x: &Extension, value: &str, owner: &str) {
        if x.ns.is_empty() || x.ns == SCXML_NS || x.ns == XMLNS_NS {
            self.problems.push(format!(
                "extension attribute `{}` on <{owner}> has namespace `{}`, which cannot carry an extension",
                x.local, x.ns
            ));
            return;
        }
        if !is_ncname(&x.local) {
            self.problems.push(format!(
                "extension attribute name `{}` on <{owner}> is not an XML NCName",
                x.local
            ));
            return;
        }
        let Some(prefix) = self.prefix_of(&x.ns).map(str::to_owned) else {
            return;
        };
        self.attr(&format!("{prefix}:{}", x.local), value, owner);
    }

    fn close_empty(&mut self) {
        self.out.push_str("/>\n");
    }

    fn close_open(&mut self) {
        self.out.push_str(">\n");
        self.indent += 1;
    }

    fn end(&mut self, name: &str) {
        self.indent -= 1;
        self.line_start();
        self.out.push_str("</");
        self.out.push_str(name);
        self.out.push_str(">\n");
    }

    /// Write an element whose children are `items`, merged with the element extensions of
    /// `exts` at their positions.
    fn element(
        &mut self,
        name: &str,
        attrs: &[(&str, Option<&str>)],
        exts: &[Extension],
        items: Vec<Item<'_>>,
    ) {
        self.open(name, attrs, exts);
        let items = merge(items, exts);
        if items.is_empty() {
            self.close_empty();
            return;
        }
        self.close_open();
        for item in items {
            self.item(item);
        }
        self.end(name);
    }

    /// An element whose content is inline text or XML (`data`, `assign`, `content`).
    fn text_element(
        &mut self,
        name: &str,
        attrs: &[(&str, Option<&str>)],
        exts: &[Extension],
        body: Option<&str>,
    ) {
        self.open(name, attrs, exts);
        match body {
            None => self.close_empty(),
            Some(b) => {
                self.out.push('>');
                if self.check_chars(b, &format!("content of <{name}>")) {
                    if looks_like_xml(b) {
                        self.out.push_str(b);
                    } else {
                        escape_text(b, &mut self.out);
                    }
                }
                self.out.push_str("</");
                self.out.push_str(name);
                self.out.push_str(">\n");
            }
        }
    }

    fn raw_element(&mut self, x: &Extension, owner: &str) {
        match &x.value {
            ExtensionValue::Element(xml) => {
                if !self.check_chars(xml, &format!("extension element `{}`", x.local)) {
                    return;
                }
                if !is_single_element(xml) {
                    self.problems.push(format!(
                        "extension element `{}` in <{owner}> is not exactly one well-formed XML element",
                        x.local
                    ));
                    return;
                }
                self.line_start();
                self.out.push_str(xml);
                self.out.push('\n');
            }
            ExtensionValue::Attribute(_) => self.problems.push(format!(
                "extension `{}` in <{owner}> stands where an element is required but is an attribute",
                x.local
            )),
        }
    }

    fn item(&mut self, item: Item<'_>) {
        match item {
            Item::State(s) => self.state(s),
            Item::Datamodel(d) => {
                self.line_start();
                self.out.push_str("<datamodel>\n");
                self.indent += 1;
                for x in d {
                    self.data(x);
                }
                self.end("datamodel");
            }
            Item::Script(s) => self.script(s, &[]),
            Item::Onentry(b) => self.block_element("onentry", b),
            Item::Onexit(b) => self.block_element("onexit", b),
            Item::Finalize(b) => self.block_element("finalize", b),
            Item::Transition(t) => self.transition(t),
            Item::Initial(i) => self.element(
                "initial",
                &[],
                &i.extensions,
                vec![Item::Transition(&i.transition)],
            ),
            Item::Invoke(i) => self.invoke(i),
            Item::DoneData(d, types) => {
                let mut items: Vec<Item<'_>> = Vec::new();
                if let Some(c) = &d.content {
                    items.push(Item::Content(c));
                }
                items.extend(params(&d.params, &types));
                self.element("donedata", &[], &[], items);
            }
            Item::Param(p, ty) => {
                self.open(
                    "param",
                    &[
                        ("name", Some(&p.name)),
                        ("expr", p.expr.as_deref()),
                        ("location", p.location.as_deref()),
                    ],
                    &[],
                );
                if let Some(t) = ty {
                    let prefix = self.prefix_of(VT_NS).unwrap_or("vt").to_owned();
                    self.attr(&format!("{prefix}:type"), &t, "param");
                }
                self.close_empty();
            }
            Item::Content(c) => {
                self.text_element(
                    "content",
                    &[("expr", c.expr.as_deref())],
                    &[],
                    c.body.as_deref(),
                );
            }
            Item::Ext(x) => self.raw_element(x, "element"),
        }
    }

    // ---------------------------------------------------------------- chart structure

    fn chart(&mut self, c: &Chart) {
        self.out
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<scxml xmlns=\"");
        self.out.push_str(SCXML_NS);
        self.out.push('"');
        for (ns, prefix) in self.prefixes.clone() {
            if prefix == "xml" {
                continue;
            }
            self.out.push_str(" xmlns:");
            self.out.push_str(&prefix);
            self.out.push_str("=\"");
            if self.check_chars(&ns, "extension namespace") {
                escape_attr(&ns, &mut self.out);
            }
            self.out.push('"');
        }
        self.out.push_str(" version=\"1.0\"");
        let initial = join(&c.initial);
        let datamodel =
            (c.datamodel_explicit || c.datamodel != "null").then_some(c.datamodel.as_str());
        let binding =
            (c.binding_explicit || c.binding == Binding::Late).then_some(match c.binding {
                Binding::Early => "early",
                Binding::Late => "late",
            });
        for (k, v) in [
            ("name", c.name.as_deref()),
            ("datamodel", datamodel),
            ("binding", binding),
            ("initial", initial.as_deref()),
        ] {
            if let Some(v) = v {
                self.attr(k, v, "scxml");
            }
        }
        for x in &c.extensions {
            if let ExtensionValue::Attribute(v) = &x.value {
                self.ext_attr(x, v, "scxml");
            }
        }
        let mut items: Vec<Item<'_>> = Vec::new();
        if !c.data.is_empty() {
            items.push(Item::Datamodel(&c.data));
        }
        if let Some(s) = &c.script {
            items.push(Item::Script(s));
        }
        items.extend(c.states.iter().map(Item::State));
        let items = merge(items, &c.extensions);
        if items.is_empty() {
            self.close_empty();
            return;
        }
        self.close_open();
        for item in items {
            self.item(item);
        }
        self.end("scxml");
    }

    fn state(&mut self, s: &State) {
        let name = match s.element {
            StateElement::State => "state",
            StateElement::Parallel => "parallel",
            StateElement::Final => "final",
            StateElement::History(_) => "history",
        };
        let write_id = !s.id.is_empty() && (!s.id_generated || self.referenced.contains(&s.id));
        let initial = join(&s.initial);
        let ty = match s.element {
            StateElement::History(HistoryType::Deep) => Some("deep"),
            _ => None,
        };
        let mut items: Vec<Item<'_>> = Vec::new();
        if let Some(i) = &s.initial_element {
            items.push(Item::Initial(i));
        }
        if !s.data.is_empty() {
            items.push(Item::Datamodel(&s.data));
        }
        items.extend(s.onentry.iter().map(|b| Item::Onentry(b)));
        items.extend(s.onexit.iter().map(|b| Item::Onexit(b)));
        items.extend(s.transitions.iter().map(Item::Transition));
        items.extend(s.children.iter().map(Item::State));
        items.extend(s.invokes.iter().map(Item::Invoke));
        let no_params: &[Param] = &[];
        let dd_params = s.donedata.as_ref().map_or(no_params, |d| &d.params);
        let (exts, types) = split_param_types(&s.extensions, dd_params);
        if let Some(d) = &s.donedata {
            items.push(Item::DoneData(d, types));
        }
        self.element(
            name,
            &[
                ("id", write_id.then_some(s.id.as_str())),
                ("initial", initial.as_deref()),
                ("type", ty),
            ],
            &exts,
            items,
        );
    }

    fn transition(&mut self, t: &Transition) {
        let events = join(&t.events);
        let targets = join(&t.targets);
        let ty = (t.kind_explicit || t.kind == TransitionType::Internal).then_some(match t.kind {
            TransitionType::External => "external",
            TransitionType::Internal => "internal",
        });
        self.no_element_extensions(&t.extensions, "transition");
        self.open(
            "transition",
            &[
                ("event", events.as_deref()),
                ("cond", t.cond.as_deref()),
                ("target", targets.as_deref()),
                ("type", ty),
            ],
            &t.extensions,
        );
        self.block_body("transition", &t.body);
    }

    fn data(&mut self, d: &Data) {
        self.no_element_extensions(&d.extensions, "data");
        let body = if d.src.is_some() {
            None
        } else {
            d.content.as_deref()
        };
        self.text_element(
            "data",
            &[
                ("id", Some(&d.id)),
                ("src", d.src.as_deref()),
                ("expr", d.expr.as_deref()),
            ],
            &d.extensions,
            body,
        );
    }

    fn script(&mut self, s: &Script, exts: &[Extension]) {
        self.open("script", &[("src", s.src.as_deref())], exts);
        let body = if s.src.is_some() {
            None
        } else {
            s.body.as_deref()
        };
        match body {
            None => self.close_empty(),
            Some(b) => {
                self.out.push('>');
                if self.check_chars(b, "script body") {
                    if (b.contains('<') || b.contains('&'))
                        && !b.contains("]]>")
                        && !b.contains('\r')
                    {
                        self.out.push_str("<![CDATA[");
                        self.out.push_str(b);
                        self.out.push_str("]]>");
                    } else {
                        escape_text(b, &mut self.out);
                    }
                }
                self.out.push_str("</script>\n");
            }
        }
    }

    fn invoke(&mut self, i: &Invoke) {
        let namelist = join(&i.namelist);
        let (exts, types) = split_param_types(&i.extensions, &i.params);
        let mut items: Vec<Item<'_>> = params(&i.params, &types);
        if i.src.is_none()
            && let Some(c) = &i.content
        {
            items.push(Item::Content(c));
        }
        if !i.finalize.is_empty() {
            items.push(Item::Finalize(&i.finalize));
        }
        self.element(
            "invoke",
            &[
                ("type", i.type_.as_deref()),
                ("typeexpr", i.typeexpr.as_deref()),
                ("src", i.src.as_deref()),
                ("srcexpr", i.srcexpr.as_deref()),
                ("id", i.id.as_deref()),
                ("idlocation", i.idlocation.as_deref()),
                ("namelist", namelist.as_deref()),
                ("autoforward", i.autoforward.then_some("true")),
            ],
            &exts,
            items,
        );
    }

    // ---------------------------------------------------------------- executable content

    fn block_element(&mut self, name: &str, b: &[Exec]) {
        self.open(name, &[], &[]);
        self.block_body(name, b);
    }

    /// Close the open start tag of `name` and write `b` as its children.
    fn block_body(&mut self, name: &str, b: &[Exec]) {
        if b.is_empty() {
            self.close_empty();
            return;
        }
        self.close_open();
        for x in b {
            self.exec(x);
        }
        self.end(name);
    }

    fn no_element_extensions(&mut self, exts: &[Extension], owner: &str) {
        for x in exts {
            if matches!(x.value, ExtensionValue::Element(_)) {
                self.problems.push(format!(
                    "extension element `{}` on <{owner}> has no place: <{owner}> cannot hold extension elements",
                    x.local
                ));
            }
        }
    }

    fn exec(&mut self, x: &Exec) {
        let exts = &x.extensions;
        match &x.kind {
            ExecKind::Raise { event } => {
                self.element("raise", &[("event", Some(event))], exts, Vec::new());
            }
            ExecKind::Send(s) => {
                let namelist = join(&s.namelist);
                let (exts, types) = split_param_types(exts, &s.params);
                let mut items: Vec<Item<'_>> = params(&s.params, &types);
                if let Some(c) = &s.content {
                    items.push(Item::Content(c));
                }
                self.element(
                    "send",
                    &[
                        ("event", s.event.as_deref()),
                        ("eventexpr", s.eventexpr.as_deref()),
                        ("target", s.target.as_deref()),
                        ("targetexpr", s.targetexpr.as_deref()),
                        ("type", s.type_.as_deref()),
                        ("typeexpr", s.typeexpr.as_deref()),
                        ("id", s.id.as_deref()),
                        ("idlocation", s.idlocation.as_deref()),
                        ("delay", s.delay.as_deref()),
                        ("delayexpr", s.delayexpr.as_deref()),
                        ("namelist", namelist.as_deref()),
                    ],
                    &exts,
                    items,
                );
            }
            ExecKind::Cancel { sendid, sendidexpr } => self.element(
                "cancel",
                &[
                    ("sendid", sendid.as_deref()),
                    ("sendidexpr", sendidexpr.as_deref()),
                ],
                exts,
                Vec::new(),
            ),
            ExecKind::Log { label, expr } => self.element(
                "log",
                &[("label", label.as_deref()), ("expr", expr.as_deref())],
                exts,
                Vec::new(),
            ),
            ExecKind::Assign {
                location,
                expr,
                content,
            } => {
                self.no_element_extensions(exts, "assign");
                self.text_element(
                    "assign",
                    &[("location", Some(location)), ("expr", expr.as_deref())],
                    exts,
                    content.as_deref(),
                );
            }
            ExecKind::Script(s) => {
                self.no_element_extensions(exts, "script");
                self.script(s, exts);
            }
            ExecKind::If { branches } => self.if_(branches, exts),
            ExecKind::Foreach {
                array,
                item,
                index,
                body,
            } => {
                self.no_element_extensions(exts, "foreach");
                self.open(
                    "foreach",
                    &[
                        ("array", Some(array)),
                        ("item", Some(item)),
                        ("index", index.as_deref()),
                    ],
                    exts,
                );
                self.block_body("foreach", body);
            }
            ExecKind::Extension(e) => {
                if !exts.is_empty() {
                    self.problems.push(format!(
                        "extension element `{}` carries extensions of its own, which cannot be attached",
                        e.local
                    ));
                }
                self.raw_element(e, "executable content");
            }
        }
    }

    fn if_(&mut self, branches: &[IfBranch], exts: &[Extension]) {
        self.no_element_extensions(exts, "if");
        let Some((first, rest)) = branches.split_first() else {
            self.problems.push("<if> without branches".to_owned());
            return;
        };
        let Some(cond) = &first.cond else {
            self.problems
                .push("<if> whose first branch has no condition".to_owned());
            return;
        };
        if rest
            .iter()
            .enumerate()
            .any(|(i, b)| b.cond.is_none() && i + 1 != rest.len())
        {
            self.problems
                .push("<if> with an <else> branch that is not last".to_owned());
            return;
        }
        self.open("if", &[("cond", Some(cond))], exts);
        if branches.iter().all(|b| b.body.is_empty()) && rest.is_empty() {
            self.close_empty();
            return;
        }
        self.close_open();
        for x in &first.body {
            self.exec(x);
        }
        for b in rest {
            match &b.cond {
                Some(c) => self.open("elseif", &[("cond", Some(c))], &[]),
                None => self.open("else", &[], &[]),
            }
            self.close_empty();
            for x in &b.body {
                self.exec(x);
            }
        }
        self.end("if");
    }
}

/// Merge extension elements into `items` at their positions. Positions are indices among all
/// element children; an extension whose position has passed is written as soon as possible.
fn merge<'c>(items: Vec<Item<'c>>, exts: &'c [Extension]) -> Vec<Item<'c>> {
    let elements: Vec<&Extension> = exts
        .iter()
        .filter(|x| matches!(x.value, ExtensionValue::Element(_)))
        .collect();
    if elements.is_empty() {
        return items;
    }
    let mut out = Vec::with_capacity(items.len() + elements.len());
    let mut next = 0;
    let mut items = items.into_iter();
    loop {
        let k = u32::try_from(out.len()).unwrap_or(u32::MAX);
        if next < elements.len() && elements[next].position <= k {
            out.push(Item::Ext(elements[next]));
            next += 1;
            continue;
        }
        match items.next() {
            Some(item) => out.push(item),
            None => {
                out.extend(elements[next..].iter().map(|x| Item::Ext(x)));
                break;
            }
        }
    }
    out
}

/// The `<param>` items of `ps`, with their `vt:type`s.
fn params<'c>(ps: &'c [Param], types: &[Option<String>]) -> Vec<Item<'c>> {
    ps.iter()
        .enumerate()
        .map(|(i, p)| Item::Param(p, types.get(i).cloned().flatten()))
        .collect()
}

/// Move an owner's `vt:paramTypes` record onto its `<param>`s as `vt:type` attributes, when
/// reading them back gives the same record: the record is in canonical form, and its fields
/// are exactly the typed parameter names in order of their first `<param>`. Otherwise the
/// record stays an attribute of the owner. Returns the owner's extensions to write and the
/// `vt:type` of each parameter.
// @tg: implements:: [[openspec:type-system#Typed SCXML Attributes]]
fn split_param_types<'e>(
    exts: &'e [Extension],
    ps: &[Param],
) -> (Cow<'e, [Extension]>, Vec<Option<String>>) {
    let none = vec![None; ps.len()];
    let Some(Ok(fields)) = param_types(exts) else {
        return (Cow::Borrowed(exts), none);
    };
    let canonical = Type::Record(fields.clone()).to_string();
    let text_is_canonical = exts.iter().any(|x| {
        x.ns == VT_NS
            && x.local == PARAM_TYPES_ATTR
            && matches!(&x.value, ExtensionValue::Attribute(v) if *v == canonical)
    });
    let mut first_order: Vec<&str> = Vec::new();
    for p in ps {
        if fields.iter().any(|f| f.name == p.name) && !first_order.contains(&p.name.as_str()) {
            first_order.push(&p.name);
        }
    }
    let same_order = first_order.len() == fields.len()
        && first_order.iter().zip(&fields).all(|(n, f)| *n == f.name);
    if !text_is_canonical || !same_order {
        return (Cow::Borrowed(exts), none);
    }
    let types = ps
        .iter()
        .map(|p| {
            fields
                .iter()
                .find(|f| f.name == p.name)
                .map(|f| f.ty.to_string())
        })
        .collect();
    let rest = exts
        .iter()
        .filter(|x| {
            !(x.ns == VT_NS
                && x.local == PARAM_TYPES_ATTR
                && matches!(x.value, ExtensionValue::Attribute(_)))
        })
        .cloned()
        .collect();
    (Cow::Owned(rest), types)
}

fn join(v: &[String]) -> Option<String> {
    (!v.is_empty()).then(|| v.join(" "))
}

/// Visit the extension list of every node of the chart.
pub(crate) fn visit_extensions(chart: &Chart, f: &mut impl FnMut(&[Extension])) {
    fn block(b: &[Exec], f: &mut impl FnMut(&[Extension])) {
        for x in b {
            f(&x.extensions);
            match &x.kind {
                ExecKind::If { branches } => {
                    for br in branches {
                        block(&br.body, f);
                    }
                }
                ExecKind::Foreach { body, .. } => block(body, f),
                _ => {}
            }
        }
    }
    fn state(s: &State, f: &mut impl FnMut(&[Extension])) {
        f(&s.extensions);
        if let Some(i) = &s.initial_element {
            f(&i.extensions);
            f(&i.transition.extensions);
            block(&i.transition.body, f);
        }
        for d in &s.data {
            f(&d.extensions);
        }
        for b in s.onentry.iter().chain(s.onexit.iter()) {
            block(b, f);
        }
        for t in &s.transitions {
            f(&t.extensions);
            block(&t.body, f);
        }
        for c in &s.children {
            state(c, f);
        }
        for i in &s.invokes {
            f(&i.extensions);
            block(&i.finalize, f);
        }
    }
    f(&chart.extensions);
    for d in &chart.data {
        f(&d.extensions);
    }
    for s in &chart.states {
        state(s, f);
    }
}

/// Whether `s` is exactly one well-formed XML element (surrounding whitespace allowed).
fn is_single_element(s: &str) -> bool {
    let mut reader = Reader::from_str(s);
    let mut depth = 0usize;
    let mut roots = 0usize;
    loop {
        match reader.read_event() {
            Err(_) => return false,
            Ok(Event::Start(_)) => {
                if depth == 0 {
                    roots += 1;
                }
                depth += 1;
            }
            Ok(Event::Empty(_)) => {
                if depth == 0 {
                    roots += 1;
                }
            }
            Ok(Event::End(_)) => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Ok(Event::Text(t)) => {
                if depth == 0 && !crate::text::is_ws(&t.xml10_content()) {
                    return false;
                }
            }
            Ok(Event::CData(_) | Event::GeneralRef(_)) if depth == 0 => return false,
            Ok(Event::Decl(_) | Event::DocType(_)) => return false,
            Ok(Event::Eof) => return depth == 0 && roots == 1,
            Ok(_) => {}
        }
    }
}
