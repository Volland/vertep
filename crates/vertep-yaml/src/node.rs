//! A small YAML node tree with source positions, built from the `saphyr-parser` event stream,
//! and the canonical block-style emitter.

use std::collections::HashMap;

use saphyr_parser::{Event, Parser, ScalarStyle, ScanError};

/// A 1-based line and column in the source; `(0, 0)` for nodes not read from a document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Pos {
    pub line: u32,
    pub column: u32,
}

/// A YAML node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    pub kind: Kind,
    pub pos: Pos,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    /// A scalar. `plain` scalars resolve `null`, booleans and numbers by the YAML 1.2 core
    /// schema; quoted and block scalars are always strings.
    Scalar {
        text: String,
        plain: bool,
    },
    Seq(Vec<Node>),
    Map(Vec<(Node, Node)>),
}

impl Node {
    pub fn new(kind: Kind) -> Self {
        Node {
            kind,
            pos: Pos::default(),
        }
    }

    /// A string scalar (quoted when emitted if needed).
    pub fn str(s: impl Into<String>) -> Self {
        Node::new(Kind::Scalar {
            text: s.into(),
            plain: false,
        })
    }

    /// A plain scalar emitted verbatim (numbers, booleans, `null`).
    pub fn raw(s: impl Into<String>) -> Self {
        Node::new(Kind::Scalar {
            text: s.into(),
            plain: true,
        })
    }

    pub fn null() -> Self {
        Node::raw("null")
    }

    pub fn bool(b: bool) -> Self {
        Node::raw(if b { "true" } else { "false" })
    }

    pub fn seq(items: Vec<Node>) -> Self {
        Node::new(Kind::Seq(items))
    }

    pub fn map(entries: Vec<(&str, Node)>) -> Self {
        Node::new(Kind::Map(
            entries
                .into_iter()
                .map(|(k, v)| (Node::str(k), v))
                .collect(),
        ))
    }

    pub fn map_owned(entries: Vec<(String, Node)>) -> Self {
        Node::new(Kind::Map(
            entries
                .into_iter()
                .map(|(k, v)| (Node::str(k), v))
                .collect(),
        ))
    }

    /// Whether this is a plain `null`, `~` or empty scalar.
    pub fn is_null(&self) -> bool {
        matches!(&self.kind, Kind::Scalar { text, plain: true }
            if matches!(text.as_str(), "" | "~" | "null" | "Null" | "NULL"))
    }

    /// The scalar text, if this is a scalar.
    pub fn scalar(&self) -> Option<&str> {
        match &self.kind {
            Kind::Scalar { text, .. } => Some(text),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(Node, Node)]> {
        match &self.kind {
            Kind::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&[Node]> {
        match &self.kind {
            Kind::Seq(s) => Some(s),
            _ => None,
        }
    }

    /// A plain boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match &self.kind {
            Kind::Scalar { text, plain: true } => match text.as_str() {
                "true" | "True" | "TRUE" => Some(true),
                "false" | "False" | "FALSE" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// A plain non-negative decimal integer.
    pub fn as_u32(&self) -> Option<u32> {
        match &self.kind {
            Kind::Scalar { text, plain: true } => {
                let t = text.strip_prefix('+').unwrap_or(text);
                if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) {
                    t.parse().ok()
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// A short description of the node kind, for messages.
    pub fn describe(&self) -> &'static str {
        match &self.kind {
            Kind::Scalar { .. } if self.is_null() => "null",
            Kind::Scalar { .. } => "a scalar",
            Kind::Seq(_) => "a list",
            Kind::Map(_) => "a mapping",
        }
    }
}

/// A YAML syntax error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    pub pos: Pos,
    pub message: String,
}

fn pos_of(m: &saphyr_parser::Marker) -> Pos {
    Pos {
        line: u32::try_from(m.line()).unwrap_or(u32::MAX),
        column: u32::try_from(m.col() + 1).unwrap_or(u32::MAX),
    }
}

fn scan_error(e: &ScanError) -> SyntaxError {
    SyntaxError {
        pos: pos_of(e.marker()),
        message: e.info().to_string(),
    }
}

enum Frame {
    Seq(Vec<Node>, Pos, usize),
    Map(Vec<(Node, Node)>, Option<Node>, Pos, usize),
}

/// Parse a single YAML 1.2 document into a node tree. Aliases are expanded; tags are
/// ignored except that a `!!str` tag makes a plain scalar a string.
pub(crate) fn parse(input: &str) -> Result<Node, SyntaxError> {
    let mut stack: Vec<Frame> = Vec::new();
    let mut anchors: HashMap<usize, Node> = HashMap::new();
    let mut root: Option<Node> = None;
    let mut documents = 0;
    for item in Parser::new_from_str(input) {
        let (event, span) = item.map_err(|e| scan_error(&e))?;
        let pos = pos_of(&span.start);
        let mut done: Option<(Node, usize)> = None;
        match event {
            Event::StreamStart | Event::StreamEnd | Event::DocumentEnd | Event::Nothing => {}
            Event::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(SyntaxError {
                        pos,
                        message: "a chart is a single YAML document; found a second document"
                            .into(),
                    });
                }
            }
            Event::Alias(id) => {
                let Some(n) = anchors.get(&id) else {
                    return Err(SyntaxError {
                        pos,
                        message: "unknown alias".into(),
                    });
                };
                let mut n = n.clone();
                n.pos = pos;
                done = Some((n, 0));
            }
            Event::Scalar(text, style, anchor, tag) => {
                let str_tag = tag
                    .as_ref()
                    .is_some_and(|t| t.is_yaml_core_schema() && t.suffix == "str");
                let plain = style == ScalarStyle::Plain && !str_tag;
                done = Some((
                    Node {
                        kind: Kind::Scalar {
                            text: text.into_owned(),
                            plain,
                        },
                        pos,
                    },
                    anchor,
                ));
            }
            Event::SequenceStart(anchor, _) => stack.push(Frame::Seq(Vec::new(), pos, anchor)),
            Event::MappingStart(anchor, _) => {
                stack.push(Frame::Map(Vec::new(), None, pos, anchor));
            }
            Event::SequenceEnd => {
                if let Some(Frame::Seq(items, p, anchor)) = stack.pop() {
                    done = Some((
                        Node {
                            kind: Kind::Seq(items),
                            pos: p,
                        },
                        anchor,
                    ));
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map(entries, _, p, anchor)) = stack.pop() {
                    done = Some((
                        Node {
                            kind: Kind::Map(entries),
                            pos: p,
                        },
                        anchor,
                    ));
                }
            }
        }
        let Some((node, anchor)) = done else { continue };
        if anchor > 0 {
            anchors.insert(anchor, node.clone());
        }
        match stack.last_mut() {
            None => root = Some(node),
            Some(Frame::Seq(items, ..)) => items.push(node),
            Some(Frame::Map(entries, key, ..)) => match key.take() {
                None => *key = Some(node),
                Some(k) => entries.push((k, node)),
            },
        }
    }
    root.ok_or(SyntaxError {
        pos: Pos { line: 1, column: 1 },
        message: "the document is empty".into(),
    })
}

// ================================================================================ emitter

/// Whether a string can be written as a plain scalar and read back as the same string under
/// the YAML 1.2 core schema (so `on` and `yes` stay plain; `null`, `true` and `false` do not).
fn plain_ok(s: &str) -> bool {
    let mut cs = s.chars();
    let first_ok = matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_');
    first_ok
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-/".contains(c))
        && !matches!(s.to_ascii_lowercase().as_str(), "null" | "true" | "false")
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            '\u{85}' | '\u{a0}' | '\u{2028}' | '\u{2029}' | '\u{feff}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn scalar(text: &str, plain: bool, out: &mut String) {
    if plain || plain_ok(text) {
        out.push_str(text);
    } else {
        quote(text, out);
    }
}

fn is_empty_collection(n: &Node) -> Option<&'static str> {
    match &n.kind {
        Kind::Map(m) if m.is_empty() => Some("{}"),
        Kind::Seq(s) if s.is_empty() => Some("[]"),
        _ => None,
    }
}

/// Emit a node as a block-style YAML document with two-space indentation.
pub(crate) fn emit(node: &Node) -> String {
    let mut out = String::new();
    match &node.kind {
        Kind::Map(m) if !m.is_empty() => emit_map(m, 0, &mut out),
        Kind::Seq(s) if !s.is_empty() => emit_seq(s, 0, &mut out),
        Kind::Scalar { text, plain } => {
            scalar(text, *plain, &mut out);
            out.push('\n');
        }
        _ => {
            out.push_str(is_empty_collection(node).unwrap_or("null"));
            out.push('\n');
        }
    }
    out
}

fn emit_value(v: &Node, indent: usize, out: &mut String) {
    if let Some(e) = is_empty_collection(v) {
        out.push(' ');
        out.push_str(e);
        out.push('\n');
        return;
    }
    match &v.kind {
        Kind::Scalar { text, plain } => {
            out.push(' ');
            scalar(text, *plain, out);
            out.push('\n');
        }
        Kind::Map(m) => {
            out.push('\n');
            emit_map(m, indent + 2, out);
        }
        Kind::Seq(s) => {
            out.push('\n');
            emit_seq(s, indent + 2, out);
        }
    }
}

fn emit_map(entries: &[(Node, Node)], indent: usize, out: &mut String) {
    for (k, v) in entries {
        out.push_str(&" ".repeat(indent));
        match &k.kind {
            Kind::Scalar { text, plain } => scalar(text, *plain, out),
            _ => out.push_str("null"),
        }
        out.push(':');
        emit_value(v, indent, out);
    }
}

fn emit_seq(items: &[Node], indent: usize, out: &mut String) {
    for item in items {
        if let Some(e) = is_empty_collection(item) {
            out.push_str(&" ".repeat(indent));
            out.push_str("- ");
            out.push_str(e);
            out.push('\n');
            continue;
        }
        match &item.kind {
            Kind::Scalar { text, plain } => {
                out.push_str(&" ".repeat(indent));
                out.push_str("- ");
                scalar(text, *plain, out);
                out.push('\n');
            }
            Kind::Map(m) => {
                let mut sub = String::new();
                emit_map(m, indent + 2, &mut sub);
                compact(&sub, indent, out);
            }
            Kind::Seq(s) => {
                let mut sub = String::new();
                emit_seq(s, indent + 2, &mut sub);
                compact(&sub, indent, out);
            }
        }
    }
}

/// Turn the first line's `indent + 2` spaces into `indent` spaces and `- `.
fn compact(sub: &str, indent: usize, out: &mut String) {
    out.push_str(&" ".repeat(indent));
    out.push_str("- ");
    out.push_str(&sub[indent + 2..]);
}
