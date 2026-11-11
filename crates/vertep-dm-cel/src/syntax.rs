//! CEL lexer and recursive-descent parser with bounded nesting.
//!
//! The grammar is the CEL language definition: conditional, `||`, `&&`, relations (`<`,
//! `<=`, `>`, `>=`, `==`, `!=`, `in`), additive, multiplicative, unary (`!`, `-`), member
//! access, indexing, calls, list and map literals and the macros `has`, `all`, `exists`,
//! `exists_one`, `map` and `filter`. Message construction (`T{f: v}`) and optional syntax
//! (`.?`, `[?]`) are not supported. Every node records its depth, and both the parser's
//! recursion and the depth of the tree are bounded, so neither parsing nor evaluation can
//! exhaust the stack ([[openspec:cel-datamodel#Guaranteed Termination]]).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::value::CelValue;

/// A parsed expression node.
#[derive(Clone, Debug)]
pub struct Expr {
    /// What the node is.
    pub kind: ExprKind,
    /// Byte offset of the node in the source.
    pub pos: usize,
    /// Height of the subtree rooted here (a leaf has depth 1).
    pub depth: u32,
}

/// Unary operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    /// `!`.
    Not,
    /// `-`.
    Neg,
}

/// Binary operators other than `&&` and `||`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `%`.
    Rem,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `in`.
    In,
}

impl BinOp {
    /// The operator's source text.
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::In => "in",
        }
    }
}

/// The comprehension macros.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Macro {
    /// `e.all(x, p)`.
    All,
    /// `e.exists(x, p)`.
    Exists,
    /// `e.exists_one(x, p)`.
    ExistsOne,
    /// `e.map(x, t)` and `e.map(x, p, t)`.
    Map,
    /// `e.filter(x, p)`.
    Filter,
}

/// Expression node kinds.
#[derive(Clone, Debug)]
pub enum ExprKind {
    /// A literal.
    Lit(CelValue),
    /// An identifier.
    Ident(Arc<str>),
    /// `operand.field`, or `has(operand.field)` when `test` is set.
    Select {
        /// The operand.
        operand: Box<Expr>,
        /// The field name.
        field: Arc<str>,
        /// Presence test (`has`).
        test: bool,
    },
    /// `operand[index]`.
    Index {
        /// The operand.
        operand: Box<Expr>,
        /// The index.
        index: Box<Expr>,
    },
    /// A function call, global (`f(a)`) or receiver-style (`t.f(a)`).
    Call {
        /// The receiver.
        target: Option<Box<Expr>>,
        /// The function name.
        name: Arc<str>,
        /// The arguments.
        args: Vec<Expr>,
    },
    /// `[a, b]`.
    List(Vec<Expr>),
    /// `{k: v}`.
    Map(Vec<(Expr, Expr)>),
    /// A unary operation.
    Unary(UnOp, Box<Expr>),
    /// A binary operation.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a && b && …`, flattened so long guard chains do not nest.
    And(Vec<Expr>),
    /// `a || b || …`, flattened likewise.
    Or(Vec<Expr>),
    /// `c ? a : b`.
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A comprehension macro.
    Comprehension {
        /// Which macro.
        kind: Macro,
        /// The iteration variable.
        var: Arc<str>,
        /// The list or map iterated.
        range: Box<Expr>,
        /// The predicate (`all`, `exists`, `exists_one`, `filter`) or transform (`map`).
        body: Box<Expr>,
        /// The filter of the three-argument `map`.
        filter: Option<Box<Expr>>,
    },
}

/// A syntax error with its byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    /// Byte offset.
    pub pos: usize,
    /// What is wrong.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Int(u64),
    Uint(u64),
    Double(f64),
    Str(String),
    Bytes(Vec<u8>),
    Ident(String),
    True,
    False,
    Null,
    In,
    P(&'static str),
    Eof,
}

const RESERVED: &[&str] = &[
    "as",
    "break",
    "const",
    "continue",
    "else",
    "for",
    "function",
    "if",
    "import",
    "let",
    "loop",
    "package",
    "namespace",
    "return",
    "var",
    "void",
    "while",
];

const PUNCT: &[&str] = &[
    "==", "!=", "<=", ">=", "&&", "||", "<", ">", "!", "+", "-", "*", "/", "%", "?", ":", ".", ",",
    "[", "]", "(", ")", "{", "}",
];

struct Lexer<'s> {
    src: &'s str,
    pos: usize,
}

fn err<T>(pos: usize, message: impl Into<String>) -> Result<T, SyntaxError> {
    Err(SyntaxError {
        pos,
        message: message.into(),
    })
}

impl<'s> Lexer<'s> {
    fn peek_byte(&self, ahead: usize) -> Option<u8> {
        self.src.as_bytes().get(self.pos + ahead).copied()
    }

    fn skip_space(&mut self) {
        loop {
            match self.peek_byte(0) {
                Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c') => self.pos += 1,
                Some(b'/') if self.peek_byte(1) == Some(b'/') => {
                    while let Some(c) = self.peek_byte(0) {
                        self.pos += 1;
                        if c == b'\n' {
                            break;
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn next(&mut self) -> Result<(Tok, usize), SyntaxError> {
        self.skip_space();
        let start = self.pos;
        let Some(c) = self.peek_byte(0) else {
            return Ok((Tok::Eof, start));
        };
        if c.is_ascii_digit()
            || (c == b'.' && self.peek_byte(1).is_some_and(|d| d.is_ascii_digit()))
        {
            return self.number().map(|t| (t, start));
        }
        if c == b'"' || c == b'\'' {
            return self.string(false, false).map(|t| (t, start));
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            // String prefixes: r, b, rb, br (any case).
            let lower = |o: usize| self.peek_byte(o).map(|b| b.to_ascii_lowercase());
            let quote = |o: usize| matches!(self.peek_byte(o), Some(b'"' | b'\''));
            match (lower(0), lower(1)) {
                (Some(b'r'), _) if quote(1) => {
                    self.pos += 1;
                    return self.string(true, false).map(|t| (t, start));
                }
                (Some(b'b'), _) if quote(1) => {
                    self.pos += 1;
                    return self.string(false, true).map(|t| (t, start));
                }
                (Some(b'r'), Some(b'b')) | (Some(b'b'), Some(b'r')) if quote(2) => {
                    self.pos += 2;
                    return self.string(true, true).map(|t| (t, start));
                }
                _ => {}
            }
            while self
                .peek_byte(0)
                .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                self.pos += 1;
            }
            let word = &self.src[start..self.pos];
            let tok = match word {
                "true" => Tok::True,
                "false" => Tok::False,
                "null" => Tok::Null,
                "in" => Tok::In,
                w if RESERVED.contains(&w) => {
                    return err(start, format!("reserved identifier `{w}`"));
                }
                w => Tok::Ident(String::from(w)),
            };
            return Ok((tok, start));
        }
        for p in PUNCT {
            if self.src[self.pos..].starts_with(p) {
                self.pos += p.len();
                return Ok((Tok::P(p), start));
            }
        }
        let ch = self.src[self.pos..].chars().next().unwrap_or('?');
        err(start, format!("unexpected character `{ch}`"))
    }

    fn number(&mut self) -> Result<Tok, SyntaxError> {
        let start = self.pos;
        let b = self.src.as_bytes();
        if b[start] == b'0' && matches!(self.peek_byte(1), Some(b'x' | b'X')) {
            self.pos += 2;
            let ds = self.pos;
            while self.peek_byte(0).is_some_and(|c| c.is_ascii_hexdigit()) {
                self.pos += 1;
            }
            let Ok(v) = u64::from_str_radix(&self.src[ds..self.pos], 16) else {
                return err(start, "invalid hexadecimal literal");
            };
            return self.int_suffix(v);
        }
        let mut is_double = false;
        while self.peek_byte(0).is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek_byte(0) == Some(b'.') && self.peek_byte(1).is_some_and(|c| c.is_ascii_digit())
        {
            is_double = true;
            self.pos += 1;
            while self.peek_byte(0).is_some_and(|c| c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek_byte(0), Some(b'e' | b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek_byte(0), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.peek_byte(0).is_some_and(|c| c.is_ascii_digit()) {
                is_double = true;
                while self.peek_byte(0).is_some_and(|c| c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        let text = &self.src[start..self.pos];
        if is_double {
            return match text.parse::<f64>() {
                Ok(d) if d.is_finite() => Ok(Tok::Double(d)),
                _ => err(start, "invalid double literal"),
            };
        }
        let Ok(v) = text.parse::<u64>() else {
            return err(start, "integer literal out of range");
        };
        self.int_suffix(v)
    }

    fn int_suffix(&mut self, v: u64) -> Result<Tok, SyntaxError> {
        if matches!(self.peek_byte(0), Some(b'u' | b'U')) {
            self.pos += 1;
            return Ok(Tok::Uint(v));
        }
        Ok(Tok::Int(v))
    }

    fn string(&mut self, raw: bool, bytes: bool) -> Result<Tok, SyntaxError> {
        let start = self.pos;
        let q = self.src.as_bytes()[start];
        let triple = self.peek_byte(1) == Some(q) && self.peek_byte(2) == Some(q);
        self.pos += if triple { 3 } else { 1 };
        let mut out: Vec<u8> = Vec::new();
        loop {
            let rest = &self.src[self.pos..];
            let Some(ch) = rest.chars().next() else {
                return err(start, "unterminated string literal");
            };
            if ch as u32 == u32::from(q) {
                if !triple {
                    self.pos += 1;
                    break;
                }
                if rest.as_bytes().get(1) == Some(&q) && rest.as_bytes().get(2) == Some(&q) {
                    self.pos += 3;
                    break;
                }
            }
            if !triple && (ch == '\n' || ch == '\r') {
                return err(self.pos, "newline in string literal");
            }
            if ch == '\\' && !raw {
                self.escape(&mut out, bytes)?;
                continue;
            }
            let mut buf = [0u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            self.pos += ch.len_utf8();
        }
        if bytes {
            Ok(Tok::Bytes(out))
        } else {
            match String::from_utf8(out) {
                Ok(s) => Ok(Tok::Str(s)),
                Err(_) => err(start, "string literal is not valid UTF-8"),
            }
        }
    }

    fn escape(&mut self, out: &mut Vec<u8>, bytes: bool) -> Result<(), SyntaxError> {
        let at = self.pos;
        self.pos += 1;
        let Some(c) = self.peek_byte(0) else {
            return err(at, "unterminated escape");
        };
        self.pos += 1;
        let simple = match c {
            b'a' => Some(7u8),
            b'b' => Some(8),
            b'f' => Some(12),
            b'n' => Some(b'\n'),
            b'r' => Some(b'\r'),
            b't' => Some(b'\t'),
            b'v' => Some(11),
            b'\\' | b'\'' | b'"' | b'`' | b'?' => Some(c),
            _ => None,
        };
        if let Some(b) = simple {
            out.push(b);
            return Ok(());
        }
        let hex = |lx: &mut Lexer<'_>, n: usize| -> Result<u32, SyntaxError> {
            let s = lx.src.get(lx.pos..lx.pos + n).unwrap_or("");
            if s.len() != n {
                return err(at, "truncated escape");
            }
            let v = u32::from_str_radix(s, 16).map_err(|_| SyntaxError {
                pos: at,
                message: String::from("invalid hexadecimal escape"),
            })?;
            lx.pos += n;
            Ok(v)
        };
        let code = match c {
            b'x' | b'X' => {
                let v = hex(self, 2)?;
                if bytes {
                    out.push(v as u8);
                    return Ok(());
                }
                v
            }
            b'u' => hex(self, 4)?,
            b'U' => hex(self, 8)?,
            b'0'..=b'3' => {
                let s = self.src.get(self.pos - 1..self.pos + 2).unwrap_or("");
                let Ok(v) = u32::from_str_radix(s, 8) else {
                    return err(at, "invalid octal escape");
                };
                self.pos += 2;
                if bytes {
                    out.push(v as u8);
                    return Ok(());
                }
                v
            }
            _ => return err(at, "invalid escape sequence"),
        };
        let Some(ch) = char::from_u32(code) else {
            return err(at, "escape is not a Unicode scalar value");
        };
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        Ok(())
    }
}

/// Parse `src` into an expression whose depth, and the parser's own recursion, stay within
/// `max_depth`.
// @tg: implements:: [[openspec:cel-datamodel#Guaranteed Termination]]
pub fn parse(src: &str, max_depth: u32) -> Result<Expr, SyntaxError> {
    let mut p = Parser {
        lx: Lexer { src, pos: 0 },
        tok: Tok::Eof,
        tok_pos: 0,
        recursion: 0,
        max_depth,
    };
    p.advance()?;
    let e = p.expr()?;
    if p.tok != Tok::Eof {
        return err(
            p.tok_pos,
            "unexpected token after the end of the expression",
        );
    }
    Ok(e)
}

struct Parser<'s> {
    lx: Lexer<'s>,
    tok: Tok,
    tok_pos: usize,
    recursion: u32,
    max_depth: u32,
}

impl Parser<'_> {
    fn advance(&mut self) -> Result<Tok, SyntaxError> {
        let (t, p) = self.lx.next()?;
        self.tok_pos = p;
        Ok(core::mem::replace(&mut self.tok, t))
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.tok, Tok::P(q) if q == p)
    }

    fn expect(&mut self, p: &str) -> Result<(), SyntaxError> {
        if self.is(p) {
            self.advance()?;
            Ok(())
        } else {
            err(self.tok_pos, format!("expected `{p}`"))
        }
    }

    fn node(&self, kind: ExprKind, pos: usize) -> Result<Expr, SyntaxError> {
        let child = match &kind {
            ExprKind::Lit(_) | ExprKind::Ident(_) => 0,
            ExprKind::Select { operand, .. } => operand.depth,
            ExprKind::Index { operand, index } => operand.depth.max(index.depth),
            ExprKind::Call { target, args, .. } => args
                .iter()
                .map(|a| a.depth)
                .chain(target.iter().map(|t| t.depth))
                .max()
                .unwrap_or(0),
            ExprKind::List(items) | ExprKind::And(items) | ExprKind::Or(items) => {
                items.iter().map(|a| a.depth).max().unwrap_or(0)
            }
            ExprKind::Map(entries) => entries
                .iter()
                .map(|(k, v)| k.depth.max(v.depth))
                .max()
                .unwrap_or(0),
            ExprKind::Unary(_, e) => e.depth,
            ExprKind::Binary(_, a, b) => a.depth.max(b.depth),
            ExprKind::Cond(a, b, c) => a.depth.max(b.depth).max(c.depth),
            ExprKind::Comprehension {
                range,
                body,
                filter,
                ..
            } => range
                .depth
                .max(body.depth)
                .max(filter.as_ref().map_or(0, |f| f.depth)),
        };
        let depth = child + 1;
        if depth > self.max_depth {
            return err(
                pos,
                format!("expression nesting exceeds the limit of {}", self.max_depth),
            );
        }
        Ok(Expr { kind, pos, depth })
    }

    fn enter(&mut self) -> Result<(), SyntaxError> {
        self.recursion += 1;
        if self.recursion > self.max_depth {
            return err(
                self.tok_pos,
                format!("expression nesting exceeds the limit of {}", self.max_depth),
            );
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<Expr, SyntaxError> {
        self.enter()?;
        let pos = self.tok_pos;
        let c = self.binary(0)?;
        let r = if self.is("?") {
            self.advance()?;
            let a = self.binary(0)?;
            self.expect(":")?;
            let b = self.expr()?;
            self.node(ExprKind::Cond(Box::new(c), Box::new(a), Box::new(b)), pos)
        } else {
            Ok(c)
        };
        self.recursion -= 1;
        r
    }

    /// The binary operator at the current token and its precedence (higher binds tighter).
    fn binop(&self) -> Option<(u8, Option<BinOp>)> {
        Some(match &self.tok {
            Tok::P("||") => (1, None),
            Tok::P("&&") => (2, None),
            Tok::P("<") => (3, Some(BinOp::Lt)),
            Tok::P("<=") => (3, Some(BinOp::Le)),
            Tok::P(">") => (3, Some(BinOp::Gt)),
            Tok::P(">=") => (3, Some(BinOp::Ge)),
            Tok::P("==") => (3, Some(BinOp::Eq)),
            Tok::P("!=") => (3, Some(BinOp::Ne)),
            Tok::In => (3, Some(BinOp::In)),
            Tok::P("+") => (4, Some(BinOp::Add)),
            Tok::P("-") => (4, Some(BinOp::Sub)),
            Tok::P("*") => (5, Some(BinOp::Mul)),
            Tok::P("/") => (5, Some(BinOp::Div)),
            Tok::P("%") => (5, Some(BinOp::Rem)),
            _ => return None,
        })
    }

    /// Precedence climbing over the left-associative binary operators: one stack frame per
    /// precedence level actually nested, not one per grammar rule.
    fn binary(&mut self, min: u8) -> Result<Expr, SyntaxError> {
        let mut l = self.unary()?;
        while let Some((prec, op)) = self.binop() {
            if prec <= min {
                break;
            }
            let pos = self.tok_pos;
            self.advance()?;
            let r = self.binary(prec)?;
            let kind = match (prec, op) {
                (1, _) => match l.kind {
                    ExprKind::Or(mut items) => {
                        items.push(r);
                        ExprKind::Or(items)
                    }
                    _ => ExprKind::Or(Vec::from([l, r])),
                },
                (2, _) => match l.kind {
                    ExprKind::And(mut items) => {
                        items.push(r);
                        ExprKind::And(items)
                    }
                    _ => ExprKind::And(Vec::from([l, r])),
                },
                (_, Some(op)) => ExprKind::Binary(op, Box::new(l), Box::new(r)),
                (_, None) => return err(pos, "unknown operator"),
            };
            l = self.node(kind, pos)?;
        }
        Ok(l)
    }

    fn unary(&mut self) -> Result<Expr, SyntaxError> {
        let pos = self.tok_pos;
        if self.is("!") {
            self.enter()?;
            self.advance()?;
            let e = self.unary()?;
            self.recursion -= 1;
            return self.node(ExprKind::Unary(UnOp::Not, Box::new(e)), pos);
        }
        if self.is("-") {
            self.enter()?;
            self.advance()?;
            // Fold `-<int literal>` so that the minimum int64 is expressible.
            if let Tok::Int(v) = self.tok {
                self.advance()?;
                let lit = if v == 1u64 << 63 {
                    i64::MIN
                } else {
                    match i64::try_from(v) {
                        Ok(i) => -i,
                        Err(_) => return err(pos, "integer literal out of range"),
                    }
                };
                let e = self.node(ExprKind::Lit(CelValue::Int(lit)), pos)?;
                let e = self.member_suffix(e)?;
                self.recursion -= 1;
                return Ok(e);
            }
            let e = self.unary()?;
            self.recursion -= 1;
            return self.node(ExprKind::Unary(UnOp::Neg, Box::new(e)), pos);
        }
        self.member()
    }

    fn member(&mut self) -> Result<Expr, SyntaxError> {
        let e = self.primary()?;
        self.member_suffix(e)
    }

    fn member_suffix(&mut self, mut e: Expr) -> Result<Expr, SyntaxError> {
        loop {
            let pos = self.tok_pos;
            if self.is(".") {
                self.advance()?;
                let Tok::Ident(name) = self.advance()? else {
                    return err(pos, "expected a field or method name after `.`");
                };
                if self.is("(") {
                    self.advance()?;
                    let args = self.args(")")?;
                    e = self.method(e, &name, args, pos)?;
                } else {
                    e = self.node(
                        ExprKind::Select {
                            operand: Box::new(e),
                            field: Arc::from(name.as_str()),
                            test: false,
                        },
                        pos,
                    )?;
                }
            } else if self.is("[") {
                self.advance()?;
                let i = self.expr()?;
                self.expect("]")?;
                e = self.node(
                    ExprKind::Index {
                        operand: Box::new(e),
                        index: Box::new(i),
                    },
                    pos,
                )?;
            } else {
                return Ok(e);
            }
        }
    }

    fn method(
        &mut self,
        target: Expr,
        name: &str,
        mut args: Vec<Expr>,
        pos: usize,
    ) -> Result<Expr, SyntaxError> {
        let kind = match (name, args.len()) {
            ("all", 2) => Some(Macro::All),
            ("exists", 2) => Some(Macro::Exists),
            ("exists_one", 2) => Some(Macro::ExistsOne),
            ("filter", 2) => Some(Macro::Filter),
            ("map", 2 | 3) => Some(Macro::Map),
            _ => None,
        };
        if let Some(kind) = kind {
            let ExprKind::Ident(var) = &args[0].kind else {
                return err(
                    args[0].pos,
                    format!("{name}() macro variable must be a simple name"),
                );
            };
            let var = var.clone();
            let body = args.pop().map(Box::new);
            let filter = if args.len() == 2 {
                args.pop().map(Box::new)
            } else {
                None
            };
            let Some(body) = body else {
                return err(pos, "missing macro body");
            };
            return self.node(
                ExprKind::Comprehension {
                    kind,
                    var,
                    range: Box::new(target),
                    body,
                    filter,
                },
                pos,
            );
        }
        self.node(
            ExprKind::Call {
                target: Some(Box::new(target)),
                name: Arc::from(name),
                args,
            },
            pos,
        )
    }

    fn args(&mut self, close: &str) -> Result<Vec<Expr>, SyntaxError> {
        let mut out = Vec::new();
        if self.is(close) {
            self.advance()?;
            return Ok(out);
        }
        loop {
            out.push(self.expr()?);
            if self.is(",") {
                self.advance()?;
                continue;
            }
            self.expect(close)?;
            return Ok(out);
        }
    }

    fn primary(&mut self) -> Result<Expr, SyntaxError> {
        let pos = self.tok_pos;
        match self.advance()? {
            Tok::Int(v) => match i64::try_from(v) {
                Ok(i) => self.node(ExprKind::Lit(CelValue::Int(i)), pos),
                Err(_) => err(pos, "integer literal out of range"),
            },
            Tok::Uint(v) => self.node(ExprKind::Lit(CelValue::Uint(v)), pos),
            Tok::Double(d) => self.node(ExprKind::Lit(CelValue::Double(d)), pos),
            Tok::Str(s) => self.node(ExprKind::Lit(CelValue::str(&s)), pos),
            Tok::Bytes(b) => self.node(ExprKind::Lit(CelValue::Bytes(Arc::from(b))), pos),
            Tok::True => self.node(ExprKind::Lit(CelValue::Bool(true)), pos),
            Tok::False => self.node(ExprKind::Lit(CelValue::Bool(false)), pos),
            Tok::Null => self.node(ExprKind::Lit(CelValue::Null), pos),
            Tok::P(".") => {
                // A leading dot names the root scope; there are no packages, so it is the
                // plain identifier.
                let Tok::Ident(name) = self.advance()? else {
                    return err(pos, "expected an identifier after a leading `.`");
                };
                self.ident(name, pos)
            }
            Tok::Ident(name) => self.ident(name, pos),
            Tok::P("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::P("[") => {
                let items = self.args("]")?;
                self.node(ExprKind::List(items), pos)
            }
            Tok::P("{") => {
                let mut entries = Vec::new();
                if self.is("}") {
                    self.advance()?;
                } else {
                    loop {
                        let k = self.expr()?;
                        self.expect(":")?;
                        let v = self.expr()?;
                        entries.push((k, v));
                        if self.is(",") {
                            self.advance()?;
                            if self.is("}") {
                                self.advance()?;
                                break;
                            }
                            continue;
                        }
                        self.expect("}")?;
                        break;
                    }
                }
                self.node(ExprKind::Map(entries), pos)
            }
            Tok::Eof => err(pos, "unexpected end of expression"),
            t => err(pos, format!("unexpected token {}", describe(&t))),
        }
    }

    fn ident(&mut self, name: String, pos: usize) -> Result<Expr, SyntaxError> {
        if self.is("(") {
            self.advance()?;
            let args = self.args(")")?;
            if name == "has" {
                let [arg] = <[Expr; 1]>::try_from(args).map_err(|_| SyntaxError {
                    pos,
                    message: String::from("has() takes exactly one argument"),
                })?;
                let ExprKind::Select { operand, field, .. } = arg.kind else {
                    return err(
                        pos,
                        "has() argument must be a field selection such as `a.b`",
                    );
                };
                return self.node(
                    ExprKind::Select {
                        operand,
                        field,
                        test: true,
                    },
                    pos,
                );
            }
            return self.node(
                ExprKind::Call {
                    target: None,
                    name: Arc::from(name.as_str()),
                    args,
                },
                pos,
            );
        }
        if self.is("{") {
            return err(pos, "message construction is not supported");
        }
        self.node(ExprKind::Ident(Arc::from(name.as_str())), pos)
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::P(p) => format!("`{p}`"),
        Tok::In => String::from("`in`"),
        _ => String::from("here"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_precedence_and_macros() {
        let e = parse("a + b * c > 1 && !d || e ? x : y", 64).unwrap();
        assert!(matches!(e.kind, ExprKind::Cond(..)));
        let e = parse("[1, 2].map(x, x * 2)", 64).unwrap();
        assert!(matches!(
            e.kind,
            ExprKind::Comprehension {
                kind: Macro::Map,
                ..
            }
        ));
        let e = parse("has(ctx.a)", 64).unwrap();
        assert!(matches!(e.kind, ExprKind::Select { test: true, .. }));
        assert!(matches!(
            parse("-9223372036854775808", 64).unwrap().kind,
            ExprKind::Lit(CelValue::Int(i64::MIN))
        ));
        assert!(parse("9223372036854775808", 64).is_err());
        assert!(parse("Msg{a: 1}", 64).is_err());
        assert!(parse("if", 64).is_err());
    }

    #[test]
    fn strings_and_bytes() {
        let lit = |s: &str| match parse(s, 8).unwrap().kind {
            ExprKind::Lit(v) => v,
            _ => panic!("not a literal"),
        };
        assert!(lit(r#""a\nb""#).cel_eq(&CelValue::str("a\nb")));
        assert!(lit(r"r'a\nb'").cel_eq(&CelValue::str("a\\nb")));
        assert!(lit(r"'é'").cel_eq(&CelValue::str("é")));
        assert!(lit(r"b'\xff'").cel_eq(&CelValue::Bytes(Arc::from(&[0xffu8][..]))));
        assert!(lit("'''multi\nline'''").cel_eq(&CelValue::str("multi\nline")));
    }

    #[test]
    fn nesting_is_bounded() {
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert!(parse(&deep, 64).is_err());
        let long = (0..10_000).map(|_| "1").collect::<Vec<_>>().join(" + ");
        assert!(parse(&long, 100).is_err());
        let guard = (0..10_000).map(|_| "a").collect::<Vec<_>>().join(" && ");
        assert_eq!(parse(&guard, 100).unwrap().depth, 2);
        let ok = format!("{}1{}", "(".repeat(50), ")".repeat(50));
        assert!(parse(&ok, 64).is_ok());
    }
}
