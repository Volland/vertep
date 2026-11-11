//! The CEL evaluator: a recursive walk over a depth-bounded tree that charges every step to
//! a [`Budget`] and reads, but never writes, its bindings.

use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;

use vertep_core::datamodel::StateQuery;

use crate::error::{CelError, ErrorKind, Limits};
use crate::funcs;
use crate::syntax::{Expr, ExprKind, Macro, UnOp};
use crate::value::{CelMap, CelValue, Key, map_get};

/// Steps and sizes consumed by one evaluation.
// @tg: implements:: [[openspec:cel-datamodel#Guaranteed Termination]]
#[derive(Debug)]
pub struct Budget {
    used: u64,
    limits: Limits,
}

impl Budget {
    /// A fresh budget under `limits`.
    pub fn new(limits: Limits) -> Self {
        Budget { used: 0, limits }
    }

    /// Steps used so far.
    pub fn used(&self) -> u64 {
        self.used
    }

    /// Charge `n` steps.
    pub fn step(&mut self, n: u64) -> Result<(), CelError> {
        self.used = self.used.saturating_add(n);
        if self.used > self.limits.max_steps {
            return Err(CelError::new(
                ErrorKind::StepLimit,
                format!(
                    "evaluation step limit exceeded: more than {} steps",
                    self.limits.max_steps
                ),
            ));
        }
        Ok(())
    }

    /// Fail when a produced value of size `n` exceeds the value size limit.
    pub fn check_size(&self, n: usize) -> Result<(), CelError> {
        if n > self.limits.max_value_size {
            return Err(CelError::new(
                ErrorKind::SizeLimit,
                format!(
                    "value size limit exceeded: {n} is more than {}",
                    self.limits.max_value_size
                ),
            ));
        }
        Ok(())
    }

    /// Fail when a produced or stored value is larger or deeper than the limits allow.
    pub fn check_value(&self, v: &CelValue) -> Result<(), CelError> {
        let (size, depth) = v.measure(self.limits.max_value_size);
        self.check_size(size)?;
        if depth > self.limits.max_value_depth {
            return Err(CelError::new(
                ErrorKind::SizeLimit,
                format!(
                    "value nesting limit exceeded: depth {depth} is more than {}",
                    self.limits.max_value_depth
                ),
            ));
        }
        Ok(())
    }

    /// The cap at which deep size counting may stop.
    pub fn size_cap(&self) -> usize {
        self.limits.max_value_size
    }
}

/// The read-only variables an expression sees.
// @tg: implements:: [[openspec:cel-datamodel#CEL Evaluation Scope]]
pub struct Scope<'a> {
    /// `ctx`.
    pub ctx: &'a CelValue,
    /// `ev` and `_event`.
    pub ev: &'a CelValue,
    /// `session`.
    pub session: &'a CelValue,
    /// `_sessionid`.
    pub sessionid: &'a CelValue,
    /// `_name`.
    pub name: &'a CelValue,
    /// `_ioprocessors`.
    pub ioprocessors: &'a CelValue,
    /// Answers `In()`.
    pub states: &'a dyn StateQuery,
}

/// Evaluate `e` in `scope` under `budget`. The scope is borrowed immutably, so evaluation
/// cannot change the datamodel.
// @tg: implements:: [[openspec:cel-datamodel#No Side Effects]]
pub fn evaluate(e: &Expr, scope: &Scope<'_>, budget: &mut Budget) -> Result<CelValue, CelError> {
    let mut ev = Evaluator {
        scope,
        locals: Vec::new(),
        budget,
    };
    ev.eval(e)
}

struct Evaluator<'a, 'b> {
    scope: &'a Scope<'a>,
    locals: Vec<(Arc<str>, CelValue)>,
    budget: &'b mut Budget,
}

fn bool_or_err(v: Result<CelValue, CelError>, op: &str) -> Result<bool, CelError> {
    match v? {
        CelValue::Bool(b) => Ok(b),
        other => Err(funcs::no_overload(op, &[&other])),
    }
}

impl Evaluator<'_, '_> {
    fn eval(&mut self, e: &Expr) -> Result<CelValue, CelError> {
        self.budget.step(1)?;
        match &e.kind {
            ExprKind::Lit(v) => Ok(v.clone()),
            ExprKind::Ident(name) => self.ident(name),
            ExprKind::Select {
                operand,
                field,
                test,
            } => {
                let v = self.eval(operand)?;
                match &v {
                    CelValue::Map(m) => {
                        let found = map_get(m, &Key::String(field.clone()));
                        if *test {
                            return Ok(CelValue::Bool(found.is_some()));
                        }
                        found.cloned().ok_or_else(|| {
                            CelError::new(ErrorKind::NoSuchKey, format!("no such key: `{field}`"))
                        })
                    }
                    other => Err(CelError::new(
                        ErrorKind::NoSuchKey,
                        format!("cannot select field `{field}` from a {}", other.type_name()),
                    )),
                }
            }
            ExprKind::Index { operand, index } => {
                let v = self.eval(operand)?;
                let i = self.eval(index)?;
                index_value(&v, &i)
            }
            ExprKind::Call { target, name, args } => {
                let mut vals = Vec::with_capacity(args.len() + 1);
                if let Some(t) = target {
                    vals.push(self.eval(t)?);
                }
                for a in args {
                    vals.push(self.eval(a)?);
                }
                if &**name == "In" && target.is_none() {
                    return match vals.as_slice() {
                        [CelValue::String(id)] => {
                            Ok(CelValue::Bool(self.scope.states.in_state(id)))
                        }
                        _ => Err(funcs::no_overload("In", &vals.iter().collect::<Vec<_>>())),
                    };
                }
                funcs::call(name, target.is_some(), &vals, self.budget)
            }
            ExprKind::List(items) => {
                self.budget.check_size(items.len())?;
                let mut out = Vec::with_capacity(items.len());
                for i in items {
                    out.push(self.eval(i)?);
                }
                Ok(CelValue::List(Arc::new(out)))
            }
            ExprKind::Map(entries) => {
                self.budget.check_size(entries.len())?;
                let mut m = CelMap::new();
                for (k, v) in entries {
                    let kv = self.eval(k)?;
                    let key = Key::from_value(&kv).ok_or_else(|| {
                        CelError::new(
                            ErrorKind::NoMatchingOverload,
                            format!("unsupported map key type {}", kv.type_name()),
                        )
                    })?;
                    let vv = self.eval(v)?;
                    if map_get(&m, &key).is_some() {
                        return Err(CelError::new(
                            ErrorKind::InvalidArgument,
                            "duplicate key in map literal",
                        ));
                    }
                    m.insert(key, vv);
                }
                Ok(CelValue::Map(Arc::new(m)))
            }
            ExprKind::Unary(UnOp::Not, x) => match self.eval(x)? {
                CelValue::Bool(b) => Ok(CelValue::Bool(!b)),
                other => Err(funcs::no_overload("!_", &[&other])),
            },
            ExprKind::Unary(UnOp::Neg, x) => funcs::negate(&self.eval(x)?),
            ExprKind::Binary(op, a, b) => {
                let x = self.eval(a)?;
                let y = self.eval(b)?;
                funcs::binary(*op, &x, &y, self.budget)
            }
            // `&&` and `||` are commutative in their error handling: a decisive operand on
            // either side wins over an error on the other (exceeded limits always propagate).
            ExprKind::And(items) => self.logic(items, false),
            ExprKind::Or(items) => self.logic(items, true),
            ExprKind::Cond(c, t, f) => {
                if bool_or_err(self.eval(c), "_?_:_")? {
                    self.eval(t)
                } else {
                    self.eval(f)
                }
            }
            ExprKind::Comprehension {
                kind,
                var,
                range,
                body,
                filter,
            } => self.comprehension(*kind, var, range, body, filter.as_deref()),
        }
    }

    fn ident(&self, name: &str) -> Result<CelValue, CelError> {
        if let Some((_, v)) = self.locals.iter().rev().find(|(n, _)| &**n == name) {
            return Ok(v.clone());
        }
        let s = self.scope;
        Ok(match name {
            "ctx" => s.ctx.clone(),
            "ev" | "_event" => s.ev.clone(),
            "session" => s.session.clone(),
            "_sessionid" => s.sessionid.clone(),
            "_name" => s.name.clone(),
            "_ioprocessors" => s.ioprocessors.clone(),
            t if funcs::TYPE_NAMES.contains(&t) => CelValue::Type(Arc::from(t)),
            _ => {
                return Err(CelError::new(
                    ErrorKind::Undeclared,
                    format!("undeclared reference to `{name}`"),
                ));
            }
        })
    }

    fn logic(&mut self, items: &[Expr], decisive: bool) -> Result<CelValue, CelError> {
        let op = if decisive { "_||_" } else { "_&&_" };
        let mut first_error: Option<CelError> = None;
        for item in items {
            match bool_or_err(self.eval(item), op) {
                Ok(x) if x == decisive => return Ok(CelValue::Bool(decisive)),
                Ok(_) => {}
                Err(e) if e.is_limit() => return Err(e),
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(CelValue::Bool(!decisive)),
        }
    }

    fn comprehension(
        &mut self,
        kind: Macro,
        var: &Arc<str>,
        range: &Expr,
        body: &Expr,
        filter: Option<&Expr>,
    ) -> Result<CelValue, CelError> {
        let r = self.eval(range)?;
        let items: Vec<CelValue> = match &r {
            CelValue::List(l) => l.iter().cloned().collect(),
            CelValue::Map(m) => m.keys().map(Key::to_value).collect(),
            other => {
                return Err(CelError::new(
                    ErrorKind::NoMatchingOverload,
                    format!("cannot iterate over a {}", other.type_name()),
                ));
            }
        };
        let mut out: Vec<CelValue> = Vec::new();
        let mut first_error: Option<CelError> = None;
        let mut count = 0u64;
        for item in items {
            self.budget.step(1)?;
            self.locals.push((var.clone(), item.clone()));
            let r = self.iteration(kind, body, filter, &item, &mut out);
            self.locals.pop();
            match r {
                Err(e) if e.is_limit() => return Err(e),
                Err(e) => match kind {
                    // `all` and `exists` absorb errors when another element decides.
                    Macro::All | Macro::Exists => {
                        first_error.get_or_insert(e);
                    }
                    _ => return Err(e),
                },
                Ok(Some(true)) => match kind {
                    Macro::Exists => return Ok(CelValue::Bool(true)),
                    Macro::ExistsOne => count += 1,
                    _ => {}
                },
                Ok(Some(false)) => {
                    if kind == Macro::All {
                        return Ok(CelValue::Bool(false));
                    }
                }
                Ok(None) => {}
            }
            if matches!(kind, Macro::Map | Macro::Filter) {
                self.budget.check_size(out.len())?;
            }
        }
        if let Some(e) = first_error {
            return Err(e);
        }
        Ok(match kind {
            Macro::All => CelValue::Bool(true),
            Macro::Exists => CelValue::Bool(false),
            Macro::ExistsOne => CelValue::Bool(count == 1),
            Macro::Map | Macro::Filter => CelValue::List(Arc::new(out)),
        })
    }

    /// One iteration: the predicate's value for `all`, `exists` and `exists_one`; `None` after
    /// pushing to `out` for `map` and `filter`.
    fn iteration(
        &mut self,
        kind: Macro,
        body: &Expr,
        filter: Option<&Expr>,
        item: &CelValue,
        out: &mut Vec<CelValue>,
    ) -> Result<Option<bool>, CelError> {
        match kind {
            Macro::All | Macro::Exists | Macro::ExistsOne => {
                bool_or_err(self.eval(body), "comprehension predicate").map(Some)
            }
            Macro::Filter => {
                if bool_or_err(self.eval(body), "filter predicate")? {
                    out.push(item.clone());
                }
                Ok(None)
            }
            Macro::Map => {
                if let Some(f) = filter
                    && !bool_or_err(self.eval(f), "map filter")?
                {
                    return Ok(None);
                }
                out.push(self.eval(body)?);
                Ok(None)
            }
        }
    }
}

/// `v[i]` for lists and maps.
pub fn index_value(v: &CelValue, i: &CelValue) -> Result<CelValue, CelError> {
    match v {
        CelValue::List(l) => {
            let ix = match i {
                CelValue::Int(n) => usize::try_from(*n).ok(),
                CelValue::Uint(n) => usize::try_from(*n).ok(),
                CelValue::Double(d)
                    if crate::value::is_integral(*d) && *d >= 0.0 && *d < 9.0e15 =>
                {
                    Some(*d as usize)
                }
                other => return Err(funcs::no_overload("_[_]", &[v, other])),
            };
            ix.and_then(|n| l.get(n)).cloned().ok_or_else(|| {
                CelError::new(
                    ErrorKind::IndexOutOfRange,
                    format!("index out of range: {}", show_key(i)),
                )
            })
        }
        CelValue::Map(m) => {
            let key = match i {
                CelValue::Double(d) if crate::value::is_integral(*d) && d.abs() < 9.2e18 => {
                    Key::Int(*d as i64)
                }
                _ => Key::from_value(i).ok_or_else(|| funcs::no_overload("_[_]", &[v, i]))?,
            };
            map_get(m, &key).cloned().ok_or_else(|| {
                CelError::new(
                    ErrorKind::NoSuchKey,
                    format!("no such key: {}", show_key(i)),
                )
            })
        }
        other => Err(funcs::no_overload("_[_]", &[other, i])),
    }
}

fn show_key(v: &CelValue) -> alloc::string::String {
    match v {
        CelValue::String(s) => format!("`{s}`"),
        CelValue::Int(i) => format!("{i}"),
        CelValue::Uint(u) => format!("{u}u"),
        CelValue::Bool(b) => format!("{b}"),
        CelValue::Double(d) => format!("{d}"),
        other => alloc::string::String::from(other.type_name()),
    }
}
