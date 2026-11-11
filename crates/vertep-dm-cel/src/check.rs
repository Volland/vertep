//! The static checker: resolves every identifier and function against the environment and,
//! for typed charts, computes the type of every node
//! ([[openspec:cel-datamodel#Static Type Checking of CEL]]).
//!
//! The checker is gradual: `dyn` matches every type and defers the check to runtime, so an
//! untyped chart (whose context and event data are `dyn`) gets name and arity checks only.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::funcs::{self, P};
use crate::syntax::{BinOp, Expr, ExprKind, Macro, UnOp};
use crate::types::{CelType, TypeEnv};
use crate::value::CelValue;

/// A static error with its byte offset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckError {
    /// Byte offset in the expression.
    pub pos: usize,
    /// What is wrong.
    pub message: String,
}

/// The variables visible to a checked expression.
pub struct CheckScope<'a> {
    /// The environment.
    pub env: &'a TypeEnv,
    /// The type of `ev.data` at this place in the chart.
    pub event_data: CelType,
}

/// Check `e`; returns its type, or every error found.
// @tg: implements:: [[openspec:cel-datamodel#Static Type Checking of CEL]]
pub fn check(e: &Expr, scope: &CheckScope<'_>) -> Result<CelType, Vec<CheckError>> {
    let mut c = Checker {
        scope,
        locals: Vec::new(),
        errors: Vec::new(),
    };
    let t = c.check(e);
    if c.errors.is_empty() {
        Ok(t)
    } else {
        Err(c.errors)
    }
}

/// The type of `ev` given the type of its `data`.
pub fn event_type(data: CelType) -> CelType {
    let opt_s = || CelType::optional(CelType::String);
    CelType::record(
        Some("event"),
        [
            ("name", CelType::String),
            ("type", CelType::String),
            ("sendid", opt_s()),
            ("origin", opt_s()),
            ("origintype", opt_s()),
            ("invokeid", opt_s()),
            ("data", data),
        ],
    )
}

/// The type of `session`.
pub fn session_type() -> CelType {
    CelType::record(
        Some("session"),
        [
            ("id", CelType::String),
            ("name", CelType::optional(CelType::String)),
        ],
    )
}

struct Checker<'a> {
    scope: &'a CheckScope<'a>,
    locals: Vec<(Arc<str>, CelType)>,
    errors: Vec<CheckError>,
}

fn join(a: &CelType, b: &CelType) -> Option<CelType> {
    match (a, b) {
        _ if a == b => Some(a.clone()),
        (CelType::Dyn, _) | (_, CelType::Dyn) => Some(CelType::Dyn),
        (CelType::Null, t) | (t, CelType::Null) => Some(match t {
            CelType::Optional(_) => t.clone(),
            _ => CelType::optional(t.clone()),
        }),
        (CelType::Optional(x), y) | (y, CelType::Optional(x)) if **x == *y => {
            Some(CelType::optional(y.clone()))
        }
        _ => None,
    }
}

/// Whether values of the two types may be compared with `==`.
fn comparable(a: &CelType, b: &CelType) -> bool {
    let (a, b) = (a.strip_optional(), b.strip_optional());
    match (a, b) {
        (CelType::Dyn, _) | (_, CelType::Dyn) => true,
        (CelType::Null, _) | (_, CelType::Null) => true,
        _ if a.is_numeric() && b.is_numeric() => true,
        (CelType::List(x), CelType::List(y)) => comparable(x, y),
        (CelType::Map(..) | CelType::Record(_) | CelType::Union(_), CelType::Map(..))
        | (CelType::Map(..), CelType::Record(_) | CelType::Union(_)) => true,
        _ => crate::types::assignable(a, b) || crate::types::assignable(b, a),
    }
}

impl Checker<'_> {
    fn error(&mut self, pos: usize, message: String) -> CelType {
        self.errors.push(CheckError { pos, message });
        CelType::Dyn
    }

    fn check(&mut self, e: &Expr) -> CelType {
        match &e.kind {
            ExprKind::Lit(v) => match v {
                CelValue::Null => CelType::Null,
                CelValue::Bool(_) => CelType::Bool,
                CelValue::Int(_) => CelType::Int,
                CelValue::Uint(_) => CelType::Uint,
                CelValue::Double(_) => CelType::Double,
                CelValue::Decimal(_) => CelType::Decimal,
                CelValue::String(_) => CelType::String,
                CelValue::Bytes(_) => CelType::Bytes,
                CelValue::Timestamp(_) => CelType::Timestamp,
                CelValue::Duration(_) => CelType::Duration,
                CelValue::Type(_) => CelType::Type,
                CelValue::List(_) => CelType::list(CelType::Dyn),
                CelValue::Map(_) => CelType::map(CelType::Dyn, CelType::Dyn),
            },
            ExprKind::Ident(name) => self.ident(name, e.pos),
            ExprKind::Select {
                operand,
                field,
                test,
            } => {
                let t = self.check(operand);
                let ft = match t.field(field) {
                    Ok(ft) => ft,
                    Err(msg) => self.error(e.pos, msg),
                };
                if *test { CelType::Bool } else { ft }
            }
            ExprKind::Index { operand, index } => {
                let t = self.check(operand);
                let i = self.check(index);
                match t.strip_optional() {
                    CelType::Dyn => CelType::Dyn,
                    CelType::List(el) => {
                        if !(P::Int.accepts(&i) || P::Uint.accepts(&i)) {
                            return self.error(e.pos, format!("list index must be int, found {i}"));
                        }
                        (**el).clone()
                    }
                    CelType::Map(k, v) => {
                        if !comparable(k, &i) {
                            return self.error(e.pos, format!("map key must be {k}, found {i}"));
                        }
                        (**v).clone()
                    }
                    CelType::Record(_) | CelType::Union(_) => match &index.kind {
                        ExprKind::Lit(CelValue::String(f)) => match t.field(f) {
                            Ok(ft) => ft,
                            Err(msg) => self.error(e.pos, msg),
                        },
                        _ => CelType::Dyn,
                    },
                    other => self.error(e.pos, format!("type {other} cannot be indexed")),
                }
            }
            ExprKind::Call { target, name, args } => {
                let mut tys = Vec::new();
                if let Some(t) = target {
                    tys.push(self.check(t));
                }
                for a in args {
                    tys.push(self.check(a));
                }
                self.call(name, target.is_some(), &tys, e.pos)
            }
            ExprKind::List(items) => {
                let mut el: Option<CelType> = None;
                for i in items {
                    let t = self.check(i);
                    el = Some(match el {
                        None => t,
                        Some(prev) => join(&prev, &t).unwrap_or(CelType::Dyn),
                    });
                }
                CelType::list(el.unwrap_or(CelType::Dyn))
            }
            ExprKind::Map(entries) => {
                let mut kt: Option<CelType> = None;
                let mut vt: Option<CelType> = None;
                for (k, v) in entries {
                    let a = self.check(k);
                    if !matches!(
                        a.strip_optional(),
                        CelType::Dyn
                            | CelType::Bool
                            | CelType::Int
                            | CelType::Uint
                            | CelType::String
                    ) {
                        self.error(k.pos, format!("unsupported map key type {a}"));
                    }
                    let b = self.check(v);
                    kt = Some(kt.map_or(a.clone(), |p| join(&p, &a).unwrap_or(CelType::Dyn)));
                    vt = Some(vt.map_or(b.clone(), |p| join(&p, &b).unwrap_or(CelType::Dyn)));
                }
                CelType::map(kt.unwrap_or(CelType::Dyn), vt.unwrap_or(CelType::Dyn))
            }
            ExprKind::Unary(UnOp::Not, x) => {
                let t = self.check(x);
                if !P::Bool.accepts(&t) {
                    return self.error(
                        e.pos,
                        format!("no matching overload for `!_` applied to ({t})"),
                    );
                }
                CelType::Bool
            }
            ExprKind::Unary(UnOp::Neg, x) => {
                let t = self.check(x);
                match t.strip_optional() {
                    CelType::Dyn => CelType::Dyn,
                    CelType::Int | CelType::Double | CelType::Decimal | CelType::Duration => {
                        t.strip_optional().clone()
                    }
                    _ => self.error(
                        e.pos,
                        format!("no matching overload for `-_` applied to ({t})"),
                    ),
                }
            }
            ExprKind::Binary(op, a, b) => {
                let x = self.check(a);
                let y = self.check(b);
                self.binary(*op, &x, &y, e.pos)
            }
            ExprKind::And(items) | ExprKind::Or(items) => {
                for side in items {
                    let t = self.check(side);
                    if !P::Bool.accepts(&t) {
                        self.error(side.pos, format!("logical operand must be bool, found {t}"));
                    }
                }
                CelType::Bool
            }
            ExprKind::Cond(c, t, f) => {
                let ct = self.check(c);
                if !P::Bool.accepts(&ct) {
                    self.error(c.pos, format!("condition must be bool, found {ct}"));
                }
                let a = self.check(t);
                let b = self.check(f);
                match join(&a, &b) {
                    Some(j) => j,
                    None if a.is_numeric() && b.is_numeric() => CelType::Dyn,
                    None => self.error(
                        e.pos,
                        format!("conditional branches have different types {a} and {b}"),
                    ),
                }
            }
            ExprKind::Comprehension {
                kind,
                var,
                range,
                body,
                filter,
            } => {
                let r = self.check(range);
                let el = match r.strip_optional() {
                    CelType::Dyn => CelType::Dyn,
                    CelType::List(el) => (**el).clone(),
                    CelType::Map(k, _) => (**k).clone(),
                    CelType::Record(_) | CelType::Union(_) => CelType::String,
                    other => self.error(range.pos, format!("cannot iterate over {other}")),
                };
                self.locals.push((var.clone(), el.clone()));
                if let Some(f) = filter {
                    let ft = self.check(f);
                    if !P::Bool.accepts(&ft) {
                        self.error(f.pos, format!("map filter must be bool, found {ft}"));
                    }
                }
                let bt = self.check(body);
                self.locals.pop();
                match kind {
                    Macro::Map => CelType::list(bt),
                    Macro::Filter | Macro::All | Macro::Exists | Macro::ExistsOne => {
                        if !P::Bool.accepts(&bt) {
                            self.error(body.pos, format!("predicate must be bool, found {bt}"));
                        }
                        if *kind == Macro::Filter {
                            CelType::list(el)
                        } else {
                            CelType::Bool
                        }
                    }
                }
            }
        }
    }

    fn ident(&mut self, name: &str, pos: usize) -> CelType {
        if let Some((_, t)) = self.locals.iter().rev().find(|(n, _)| &**n == name) {
            return t.clone();
        }
        match name {
            "ctx" => self.scope.env.context_type(),
            "ev" | "_event" => event_type(self.scope.event_data.clone()),
            "session" => session_type(),
            "_sessionid" => CelType::String,
            "_name" => CelType::optional(CelType::String),
            "_ioprocessors" => CelType::Dyn,
            t if funcs::TYPE_NAMES.contains(&t) => CelType::Type,
            _ => self.error(pos, format!("undeclared reference to `{name}`")),
        }
    }

    fn call(&mut self, name: &str, method: bool, tys: &[CelType], pos: usize) -> CelType {
        let Some(decl) = funcs::lookup(name) else {
            return self.error(pos, format!("undeclared reference to function `{name}`"));
        };
        let mut result: Option<CelType> = None;
        let mut matched = false;
        for o in decl.overloads {
            if o.method != method || o.params.len() != tys.len() {
                continue;
            }
            if o.params.iter().zip(tys).all(|(p, t)| p.accepts(t)) {
                let r = o.result.result_type();
                result = Some(match (matched, result) {
                    (false, _) | (true, None) => r,
                    (true, Some(prev)) if prev == r => prev,
                    _ => CelType::Dyn,
                });
                matched = true;
            }
        }
        match result {
            Some(r) => r,
            None => {
                let shown: Vec<String> = tys.iter().map(|t| format!("{t}")).collect();
                self.error(
                    pos,
                    format!(
                        "no matching overload for `{name}` applied to ({})",
                        shown.join(", ")
                    ),
                )
            }
        }
    }

    fn binary(&mut self, op: BinOp, a: &CelType, b: &CelType, pos: usize) -> CelType {
        use CelType as T;
        let (x, y) = (a.strip_optional(), b.strip_optional());
        let dynamic = matches!(x, T::Dyn) || matches!(y, T::Dyn);
        let bad = |c: &mut Self| {
            c.error(
                pos,
                format!(
                    "no matching overload for `_{}_` applied to ({a}, {b})",
                    op.symbol()
                ),
            )
        };
        let dec_mix = |x: &T, y: &T| {
            matches!(
                (x, y),
                (T::Decimal, T::Decimal | T::Int | T::Uint) | (T::Int | T::Uint, T::Decimal)
            )
        };
        match op {
            BinOp::Eq | BinOp::Ne => {
                if comparable(a, b) {
                    T::Bool
                } else {
                    bad(self)
                }
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let ok = dynamic
                    || (x.is_numeric() && y.is_numeric())
                    || (x == y
                        && matches!(
                            x,
                            T::Bool | T::String | T::Bytes | T::Timestamp | T::Duration
                        ));
                if ok { T::Bool } else { bad(self) }
            }
            BinOp::In => match y {
                T::Dyn => T::Bool,
                T::List(el) if comparable(el, a) => T::Bool,
                T::Map(k, _) if comparable(k, a) => T::Bool,
                T::Record(_) | T::Union(_) if P::String.accepts(a) => T::Bool,
                _ => bad(self),
            },
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {
                if dynamic {
                    return T::Dyn;
                }
                let arith = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div);
                let r = match (x, y) {
                    (T::Int, T::Int) => Some(T::Int),
                    (T::Uint, T::Uint) => Some(T::Uint),
                    (T::Double, T::Double) if arith => Some(T::Double),
                    _ if dec_mix(x, y) => Some(T::Decimal),
                    (T::String, T::String) | (T::Bytes, T::Bytes) if op == BinOp::Add => {
                        Some(x.clone())
                    }
                    (T::List(p), T::List(q)) if op == BinOp::Add => {
                        Some(T::list(join(p, q).unwrap_or(T::Dyn)))
                    }
                    (T::Timestamp, T::Duration) if matches!(op, BinOp::Add | BinOp::Sub) => {
                        Some(T::Timestamp)
                    }
                    (T::Duration, T::Timestamp) if op == BinOp::Add => Some(T::Timestamp),
                    (T::Duration, T::Duration) if matches!(op, BinOp::Add | BinOp::Sub) => {
                        Some(T::Duration)
                    }
                    (T::Timestamp, T::Timestamp) if op == BinOp::Sub => Some(T::Duration),
                    _ => None,
                };
                match r {
                    Some(t) => t,
                    None => bad(self),
                }
            }
        }
    }
}
