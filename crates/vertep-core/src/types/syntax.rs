//! The textual syntax of types and declaration blocks.
//!
//! ```text
//! type      := '|' variant ('|' variant)*          a union, leading bar allowed
//!            | variant ('|' variant)+              a union
//!            | tag atom                            a one-variant union with payload
//!            | atom
//! variant   := tag atom?                           tag is an identifier or a "quoted string"
//! atom      := bool | int | decimal | string | datetime | iri | value
//!            | list '<' type '>' | optional '<' type '>'
//!            | '{' (name ':' type (',' name ':' type)* ','?)? '}'
//!            | '(' type ')'
//!            | Name                                a named type
//! decls     := (('type' Name '=' type | 'event' event-name (':' type)?) ';')*
//! ```
//!
//! `#` starts a comment that runs to the end of the line. Identifiers are
//! `[A-Za-z_][A-Za-z0-9_]*`; field names and tags that are not identifiers are written as
//! double-quoted strings with `\"`, `\\`, `\n`, `\r`, `\t` and `\u{...}` escapes. Event names
//! are any run of characters other than whitespace, `:`, `;`, `#` and `"`, or a quoted string.
//!
//! The printer ([`core::fmt::Display`] on [`Type`] and [`Declarations`]) is canonical and
//! `parse_type(&t.to_string()) == Ok(t)` for every valid type.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use crate::ir::Span;
use crate::types::decl::{Declarations, EventDecl, TypeDecl};
use crate::types::ty::{Field, Primitive, Type, Variant};

/// A syntax error in a type expression or declaration block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// Byte offset of the error in the parsed text.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {})", self.message, self.offset)
    }
}

/// Names that cannot name a type or be an unquoted tag.
pub const RESERVED: [&str; 9] = [
    "bool", "int", "decimal", "string", "datetime", "iri", "value", "list", "optional",
];

/// Whether `name` is reserved by the type language.
pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

/// Whether `name` is an identifier `[A-Za-z_][A-Za-z0-9_]*`.
pub fn is_ident(name: &str) -> bool {
    let mut cs = name.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse a type expression such as `list<Order>` or `Card { last4: string } | Cash`.
pub fn parse_type(src: &str) -> Result<Type, SyntaxError> {
    let mut p = Parser::new(src);
    let t = p.ty()?;
    p.expect_eof()?;
    Ok(t)
}

/// Parse a declaration block such as `type Order = { id: string }; event pay: Order;`.
pub fn parse_declarations(src: &str) -> Result<Declarations, SyntaxError> {
    let mut p = Parser::new(src);
    let mut out = Declarations::default();
    loop {
        match p.peek()? {
            Tok::Eof => break,
            Tok::Ident(k) if k == "type" => {
                p.bump();
                let at = p.offset();
                let name = match p.next()? {
                    Tok::Ident(n) if !is_reserved(&n) => n,
                    Tok::Ident(n) => {
                        return Err(p.err_at(at, alloc::format!("`{n}` is reserved")));
                    }
                    _ => return Err(p.err_at(at, "expected a type name".to_string())),
                };
                p.expect('=')?;
                let ty = p.ty()?;
                out.types.push(TypeDecl {
                    name,
                    ty,
                    span: Span::NONE,
                });
            }
            Tok::Ident(k) if k == "event" => {
                p.bump();
                let name = p.event_name()?;
                let payload = if p.eat(':')? { Some(p.ty()?) } else { None };
                out.events.push(EventDecl {
                    name,
                    payload,
                    span: Span::NONE,
                });
            }
            _ => {
                let at = p.offset();
                return Err(p.err_at(at, "expected `type` or `event`".to_string()));
            }
        }
        if !p.eat(';')? && p.peek()? != Tok::Eof {
            let at = p.offset();
            return Err(p.err_at(at, "expected `;`".to_string()));
        }
    }
    Ok(out)
}

// ================================================================================ lexer

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Punct(char),
    Eof,
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    peeked: Option<(Tok, usize, usize)>,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser {
            src,
            pos: 0,
            peeked: None,
        }
    }

    fn err_at(&self, offset: usize, message: String) -> SyntaxError {
        SyntaxError { offset, message }
    }

    fn skip_ws(&mut self) {
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b == b'#' {
                while self.pos < bytes.len() && bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Offset of the next token.
    fn offset(&mut self) -> usize {
        if let Some((_, start, _)) = &self.peeked {
            return *start;
        }
        self.skip_ws();
        self.pos
    }

    fn lex(&mut self) -> Result<(Tok, usize, usize), SyntaxError> {
        self.skip_ws();
        let start = self.pos;
        let rest = &self.src[self.pos..];
        let Some(c) = rest.chars().next() else {
            return Ok((Tok::Eof, start, start));
        };
        if c.is_ascii_alphabetic() || c == '_' {
            let len = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            self.pos += len;
            return Ok((Tok::Ident(rest[..len].to_string()), start, self.pos));
        }
        if c == '"' {
            let s = self.string()?;
            return Ok((Tok::Str(s), start, self.pos));
        }
        if "{}<>()|,:;=".contains(c) {
            self.pos += 1;
            return Ok((Tok::Punct(c), start, self.pos));
        }
        Err(self.err_at(start, alloc::format!("unexpected character `{c}`")))
    }

    /// A double-quoted string starting at `self.pos`.
    fn string(&mut self) -> Result<String, SyntaxError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let rest = &self.src[self.pos..];
            let Some(c) = rest.chars().next() else {
                return Err(self.err_at(start, "unterminated string".to_string()));
            };
            self.pos += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let at = self.pos - 1;
                    let Some(e) = self.src[self.pos..].chars().next() else {
                        return Err(self.err_at(at, "unterminated escape".to_string()));
                    };
                    self.pos += e.len_utf8();
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let r = &self.src[self.pos..];
                            let close = r.find('}');
                            let ch = match (r.strip_prefix('{'), close) {
                                (Some(_), Some(close)) => u32::from_str_radix(&r[1..close], 16)
                                    .ok()
                                    .and_then(char::from_u32),
                                _ => None,
                            };
                            let Some(ch) = ch else {
                                return Err(self.err_at(at, "bad \\u{...} escape".to_string()));
                            };
                            self.pos += close.unwrap_or(0) + 1;
                            out.push(ch);
                        }
                        _ => {
                            return Err(self.err_at(at, alloc::format!("unknown escape `\\{e}`")));
                        }
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn peek(&mut self) -> Result<Tok, SyntaxError> {
        if self.peeked.is_none() {
            let t = self.lex()?;
            self.peeked = Some(t);
        }
        Ok(self
            .peeked
            .as_ref()
            .map(|t| t.0.clone())
            .unwrap_or(Tok::Eof))
    }

    fn bump(&mut self) {
        self.peeked = None;
    }

    fn next(&mut self) -> Result<Tok, SyntaxError> {
        let t = self.peek()?;
        self.bump();
        Ok(t)
    }

    fn eat(&mut self, c: char) -> Result<bool, SyntaxError> {
        if self.peek()? == Tok::Punct(c) {
            self.bump();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn expect(&mut self, c: char) -> Result<(), SyntaxError> {
        let at = self.offset();
        if self.eat(c)? {
            Ok(())
        } else {
            Err(self.err_at(at, alloc::format!("expected `{c}`")))
        }
    }

    fn expect_eof(&mut self) -> Result<(), SyntaxError> {
        let at = self.offset();
        match self.peek()? {
            Tok::Eof => Ok(()),
            _ => Err(self.err_at(at, "unexpected text after the type".to_string())),
        }
    }

    /// An event name: a quoted string or a run of characters other than whitespace, `:`, `;`,
    /// `#` and `"`.
    fn event_name(&mut self) -> Result<String, SyntaxError> {
        debug_assert!(self.peeked.is_none());
        self.skip_ws();
        let rest = &self.src[self.pos..];
        if rest.starts_with('"') {
            return self.string();
        }
        let len = rest
            .find(|c: char| c.is_whitespace() || ":;#\"".contains(c))
            .unwrap_or(rest.len());
        if len == 0 {
            return Err(self.err_at(self.pos, "expected an event name".to_string()));
        }
        self.pos += len;
        Ok(rest[..len].to_string())
    }

    fn starts_payload(&mut self) -> Result<bool, SyntaxError> {
        Ok(matches!(
            self.peek()?,
            Tok::Punct('{') | Tok::Punct('(') | Tok::Ident(_)
        ))
    }

    // ============================================================================ grammar

    fn ty(&mut self) -> Result<Type, SyntaxError> {
        if self.eat('|')? {
            let first = self.variant()?;
            return self.union_rest(Vec::from([first]));
        }
        let at = self.offset();
        match self.peek()? {
            Tok::Str(tag) => {
                self.bump();
                let v = self.variant_payload(tag)?;
                self.union_rest(Vec::from([v]))
            }
            Tok::Ident(name) if !is_reserved(&name) => {
                self.bump();
                if self.peek()? == Tok::Punct('<') {
                    let at = self.offset();
                    return Err(
                        self.err_at(at, alloc::format!("named type `{name}` takes no arguments"))
                    );
                }
                if self.peek()? == Tok::Punct('|') || self.starts_payload()? {
                    let v = self.variant_payload(name)?;
                    self.union_rest(Vec::from([v]))
                } else {
                    Ok(Type::Named(name))
                }
            }
            _ => {
                let t = self.atom()?;
                if self.peek()? == Tok::Punct('|') {
                    return Err(
                        self.err_at(at, "a union variant must start with a tag".to_string())
                    );
                }
                Ok(t)
            }
        }
    }

    fn union_rest(&mut self, mut variants: Vec<Variant>) -> Result<Type, SyntaxError> {
        while self.eat('|')? {
            variants.push(self.variant()?);
        }
        Ok(Type::Union(variants))
    }

    fn variant(&mut self) -> Result<Variant, SyntaxError> {
        let at = self.offset();
        let tag = match self.next()? {
            Tok::Str(s) => s,
            Tok::Ident(n) if !is_reserved(&n) => n,
            Tok::Ident(n) => {
                return Err(self.err_at(at, alloc::format!("`{n}` is reserved; quote the tag")));
            }
            _ => return Err(self.err_at(at, "expected a variant tag".to_string())),
        };
        self.variant_payload(tag)
    }

    fn variant_payload(&mut self, tag: String) -> Result<Variant, SyntaxError> {
        let payload = if self.starts_payload()? {
            Some(self.atom()?)
        } else {
            None
        };
        Ok(Variant { tag, payload })
    }

    fn atom(&mut self) -> Result<Type, SyntaxError> {
        let at = self.offset();
        match self.next()? {
            Tok::Punct('{') => self.record_rest(),
            Tok::Punct('(') => {
                let t = self.ty()?;
                self.expect(')')?;
                Ok(t)
            }
            Tok::Ident(n) => {
                if let Some(p) = Primitive::from_name(&n) {
                    return Ok(Type::Primitive(p));
                }
                match n.as_str() {
                    "value" => Ok(Type::Value),
                    "list" | "optional" => {
                        self.expect('<')?;
                        let inner = self.ty()?;
                        self.expect('>')?;
                        Ok(if n == "list" {
                            Type::List(Box::new(inner))
                        } else {
                            Type::Optional(Box::new(inner))
                        })
                    }
                    _ => Ok(Type::Named(n)),
                }
            }
            Tok::Eof => Err(self.err_at(at, "expected a type".to_string())),
            t => Err(self.err_at(
                at,
                alloc::format!("expected a type, found {}", describe(&t)),
            )),
        }
    }

    fn record_rest(&mut self) -> Result<Type, SyntaxError> {
        let mut fields = Vec::new();
        loop {
            if self.eat('}')? {
                return Ok(Type::Record(fields));
            }
            let at = self.offset();
            let name = match self.next()? {
                Tok::Ident(n) | Tok::Str(n) => n,
                _ => return Err(self.err_at(at, "expected a field name or `}`".to_string())),
            };
            self.expect(':')?;
            let ty = self.ty()?;
            fields.push(Field { name, ty });
            if !self.eat(',')? {
                self.expect('}')?;
                return Ok(Type::Record(fields));
            }
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(n) => alloc::format!("`{n}`"),
        Tok::Str(s) => alloc::format!("\"{s}\""),
        Tok::Punct(c) => alloc::format!("`{c}`"),
        Tok::Eof => "end of input".to_string(),
    }
}

// ================================================================================ printer

fn write_quoted(s: &str, f: &mut dyn fmt::Write) -> fmt::Result {
    f.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if c.is_control() => write!(f, "\\u{{{:x}}}", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

fn write_field_name(name: &str, f: &mut dyn fmt::Write) -> fmt::Result {
    if is_ident(name) {
        f.write_str(name)
    } else {
        write_quoted(name, f)
    }
}

fn write_tag(tag: &str, f: &mut dyn fmt::Write) -> fmt::Result {
    if is_ident(tag) && !is_reserved(tag) {
        f.write_str(tag)
    } else {
        write_quoted(tag, f)
    }
}

fn write_event_name(name: &str, f: &mut dyn fmt::Write) -> fmt::Result {
    let plain = !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || ":;#\"".contains(c));
    if plain {
        f.write_str(name)
    } else {
        write_quoted(name, f)
    }
}

fn write_variant(v: &Variant, f: &mut dyn fmt::Write) -> fmt::Result {
    write_tag(&v.tag, f)?;
    if let Some(p) = &v.payload {
        f.write_char(' ')?;
        if matches!(p, Type::Union(_)) {
            f.write_char('(')?;
            write_type(p, f)?;
            f.write_char(')')?;
        } else {
            write_type(p, f)?;
        }
    }
    Ok(())
}

/// Write `t` in the canonical textual syntax.
pub(crate) fn write_type(t: &Type, f: &mut dyn fmt::Write) -> fmt::Result {
    match t {
        Type::Primitive(p) => f.write_str(p.name()),
        Type::Value => f.write_str("value"),
        Type::Named(n) => f.write_str(n),
        Type::List(t) => {
            f.write_str("list<")?;
            write_type(t, f)?;
            f.write_char('>')
        }
        Type::Optional(t) => {
            f.write_str("optional<")?;
            write_type(t, f)?;
            f.write_char('>')
        }
        Type::Record(fields) => {
            if fields.is_empty() {
                return f.write_str("{}");
            }
            f.write_str("{ ")?;
            for (i, fd) in fields.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write_field_name(&fd.name, f)?;
                f.write_str(": ")?;
                write_type(&fd.ty, f)?;
            }
            f.write_str(" }")
        }
        Type::Union(vs) => {
            if vs.len() == 1 {
                f.write_str("| ")?;
            }
            for (i, v) in vs.iter().enumerate() {
                if i > 0 {
                    f.write_str(" | ")?;
                }
                write_variant(v, f)?;
            }
            if vs.is_empty() {
                f.write_char('|')?;
            }
            Ok(())
        }
    }
}

/// Write a declaration block in the canonical syntax: one declaration per line, types first,
/// then events, each in declaration order.
pub(crate) fn write_declarations(d: &Declarations, f: &mut dyn fmt::Write) -> fmt::Result {
    for t in &d.types {
        write!(f, "type {} = ", t.name)?;
        write_type(&t.ty, f)?;
        f.write_str(";\n")?;
    }
    for e in &d.events {
        f.write_str("event ")?;
        write_event_name(&e.name, f)?;
        if let Some(p) = &e.payload {
            f.write_str(": ")?;
            write_type(p, f)?;
        }
        f.write_str(";\n")?;
    }
    Ok(())
}
