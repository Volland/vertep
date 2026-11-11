//! The CEL standard functions and operators available to charts, declared once for the
//! checker and implemented for the evaluator.
//!
//! The environment holds the CEL standard definitions minus anything effectful or
//! non-deterministic: there is no `now()`, no randomness, no I/O and no host function, and
//! time zones are fixed offsets only ([[openspec:cel-datamodel#No Side Effects]]). The
//! additions are the `decimal` type with its `decimal()` conversion and the SCXML predicate
//! `In(stateId)`.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Ordering;

use regex_automata::nfa::thompson;
use regex_automata::nfa::thompson::pikevm::PikeVM;

use crate::decimal::{Decimal, DecimalError};
use crate::error::{CelError, ErrorKind};
use crate::eval::Budget;
use crate::syntax::BinOp;
use crate::time::{Duration, Timestamp, parse_zone};
use crate::types::CelType;
use crate::value::{CelValue, Key, map_get, num_cmp};

/// A parameter or result pattern in a function declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P {
    /// Any type.
    Any,
    /// `bool`.
    Bool,
    /// `int`.
    Int,
    /// `uint`.
    Uint,
    /// `double`.
    Double,
    /// `decimal`.
    Decimal,
    /// `string`.
    String,
    /// `bytes`.
    Bytes,
    /// `google.protobuf.Timestamp`.
    Timestamp,
    /// `google.protobuf.Duration`.
    Duration,
    /// Any list.
    List,
    /// Any map, record or union.
    Map,
    /// `type`.
    Type,
    /// `dyn` (result only).
    Dyn,
}

impl P {
    /// Whether a value of static type `t` may be passed for this pattern.
    pub fn accepts(self, t: &CelType) -> bool {
        matches!(
            (self, t.strip_optional()),
            (P::Any, _)
                | (_, CelType::Dyn)
                | (P::Bool, CelType::Bool)
                | (P::Int, CelType::Int)
                | (P::Uint, CelType::Uint)
                | (P::Double, CelType::Double)
                | (P::Decimal, CelType::Decimal)
                | (P::String, CelType::String)
                | (P::Bytes, CelType::Bytes)
                | (P::Timestamp, CelType::Timestamp)
                | (P::Duration, CelType::Duration)
                | (P::Type, CelType::Type)
                | (P::List, CelType::List(_))
                | (
                    P::Map,
                    CelType::Map(..) | CelType::Record(_) | CelType::Union(_)
                )
        )
    }

    /// The static type of a result pattern.
    pub fn result_type(self) -> CelType {
        match self {
            P::Any | P::Dyn => CelType::Dyn,
            P::Bool => CelType::Bool,
            P::Int => CelType::Int,
            P::Uint => CelType::Uint,
            P::Double => CelType::Double,
            P::Decimal => CelType::Decimal,
            P::String => CelType::String,
            P::Bytes => CelType::Bytes,
            P::Timestamp => CelType::Timestamp,
            P::Duration => CelType::Duration,
            P::List => CelType::list(CelType::Dyn),
            P::Map => CelType::map(CelType::Dyn, CelType::Dyn),
            P::Type => CelType::Type,
        }
    }
}

/// One overload: receiver-style or global, parameter patterns (the receiver first for
/// methods) and the result.
#[derive(Clone, Copy, Debug)]
pub struct Overload {
    /// Called as `target.name(args)`.
    pub method: bool,
    /// Parameter patterns.
    pub params: &'static [P],
    /// Result pattern.
    pub result: P,
}

/// A declared function.
#[derive(Clone, Copy, Debug)]
pub struct FnDecl {
    /// The function name.
    pub name: &'static str,
    /// Its overloads.
    pub overloads: &'static [Overload],
}

const fn g(params: &'static [P], result: P) -> Overload {
    Overload {
        method: false,
        params,
        result,
    }
}

const fn m(params: &'static [P], result: P) -> Overload {
    Overload {
        method: true,
        params,
        result,
    }
}

const TS_GETTER: &[Overload] = &[
    m(&[P::Timestamp], P::Int),
    m(&[P::Timestamp, P::String], P::Int),
];
const TIME_GETTER: &[Overload] = &[
    m(&[P::Timestamp], P::Int),
    m(&[P::Timestamp, P::String], P::Int),
    m(&[P::Duration], P::Int),
];

/// Every function of the CEL datamodel environment.
// @tg: implements:: [[openspec:cel-datamodel#No Side Effects]]
pub const FUNCTIONS: &[FnDecl] = &[
    FnDecl {
        name: "size",
        overloads: &[
            g(&[P::String], P::Int),
            g(&[P::Bytes], P::Int),
            g(&[P::List], P::Int),
            g(&[P::Map], P::Int),
            m(&[P::String], P::Int),
            m(&[P::Bytes], P::Int),
            m(&[P::List], P::Int),
            m(&[P::Map], P::Int),
        ],
    },
    FnDecl {
        name: "contains",
        overloads: &[m(&[P::String, P::String], P::Bool)],
    },
    FnDecl {
        name: "startsWith",
        overloads: &[m(&[P::String, P::String], P::Bool)],
    },
    FnDecl {
        name: "endsWith",
        overloads: &[m(&[P::String, P::String], P::Bool)],
    },
    FnDecl {
        name: "matches",
        overloads: &[
            m(&[P::String, P::String], P::Bool),
            g(&[P::String, P::String], P::Bool),
        ],
    },
    FnDecl {
        name: "int",
        overloads: &[
            g(&[P::Int], P::Int),
            g(&[P::Uint], P::Int),
            g(&[P::Double], P::Int),
            g(&[P::Decimal], P::Int),
            g(&[P::String], P::Int),
            g(&[P::Timestamp], P::Int),
        ],
    },
    FnDecl {
        name: "uint",
        overloads: &[
            g(&[P::Int], P::Uint),
            g(&[P::Uint], P::Uint),
            g(&[P::Double], P::Uint),
            g(&[P::Decimal], P::Uint),
            g(&[P::String], P::Uint),
        ],
    },
    FnDecl {
        name: "double",
        overloads: &[
            g(&[P::Int], P::Double),
            g(&[P::Uint], P::Double),
            g(&[P::Double], P::Double),
            g(&[P::Decimal], P::Double),
            g(&[P::String], P::Double),
        ],
    },
    FnDecl {
        name: "decimal",
        overloads: &[
            g(&[P::Int], P::Decimal),
            g(&[P::Uint], P::Decimal),
            g(&[P::Double], P::Decimal),
            g(&[P::Decimal], P::Decimal),
            g(&[P::String], P::Decimal),
        ],
    },
    FnDecl {
        name: "string",
        overloads: &[
            g(&[P::Bool], P::String),
            g(&[P::Int], P::String),
            g(&[P::Uint], P::String),
            g(&[P::Double], P::String),
            g(&[P::Decimal], P::String),
            g(&[P::String], P::String),
            g(&[P::Bytes], P::String),
            g(&[P::Timestamp], P::String),
            g(&[P::Duration], P::String),
            g(&[P::Type], P::String),
        ],
    },
    FnDecl {
        name: "bytes",
        overloads: &[g(&[P::String], P::Bytes), g(&[P::Bytes], P::Bytes)],
    },
    FnDecl {
        name: "bool",
        overloads: &[g(&[P::Bool], P::Bool), g(&[P::String], P::Bool)],
    },
    FnDecl {
        name: "timestamp",
        overloads: &[
            g(&[P::String], P::Timestamp),
            g(&[P::Int], P::Timestamp),
            g(&[P::Timestamp], P::Timestamp),
        ],
    },
    FnDecl {
        name: "duration",
        overloads: &[g(&[P::String], P::Duration), g(&[P::Duration], P::Duration)],
    },
    FnDecl {
        name: "dyn",
        overloads: &[g(&[P::Any], P::Dyn)],
    },
    FnDecl {
        name: "type",
        overloads: &[g(&[P::Any], P::Type)],
    },
    FnDecl {
        name: "In",
        overloads: &[g(&[P::String], P::Bool)],
    },
    FnDecl {
        name: "getFullYear",
        overloads: TS_GETTER,
    },
    FnDecl {
        name: "getMonth",
        overloads: TS_GETTER,
    },
    FnDecl {
        name: "getDate",
        overloads: TS_GETTER,
    },
    FnDecl {
        name: "getDayOfMonth",
        overloads: TS_GETTER,
    },
    FnDecl {
        name: "getDayOfWeek",
        overloads: TS_GETTER,
    },
    FnDecl {
        name: "getDayOfYear",
        overloads: TS_GETTER,
    },
    FnDecl {
        name: "getHours",
        overloads: TIME_GETTER,
    },
    FnDecl {
        name: "getMinutes",
        overloads: TIME_GETTER,
    },
    FnDecl {
        name: "getSeconds",
        overloads: TIME_GETTER,
    },
    FnDecl {
        name: "getMilliseconds",
        overloads: TIME_GETTER,
    },
];

/// The declaration of `name`, if the environment has such a function.
pub fn lookup(name: &str) -> Option<&'static FnDecl> {
    FUNCTIONS.iter().find(|f| f.name == name)
}

/// Names usable as type values (`type(x) == int`).
pub const TYPE_NAMES: &[&str] = &[
    "bool",
    "int",
    "uint",
    "double",
    "decimal",
    "string",
    "bytes",
    "list",
    "map",
    "null_type",
    "type",
];

pub(crate) fn no_overload(name: &str, args: &[&CelValue]) -> CelError {
    let tys: Vec<&str> = args.iter().map(|a| a.type_name()).collect();
    CelError::new(
        ErrorKind::NoMatchingOverload,
        format!(
            "no matching overload for `{name}` applied to ({})",
            tys.join(", ")
        ),
    )
}

fn overflow(what: &str) -> CelError {
    CelError::new(ErrorKind::Overflow, format!("{what} overflow"))
}

fn conversion(msg: impl Into<String>) -> CelError {
    CelError::new(ErrorKind::Conversion, msg)
}

fn dec_err(e: DecimalError) -> CelError {
    match e {
        DecimalError::DivisionByZero => {
            CelError::new(ErrorKind::DivisionByZero, "division by zero")
        }
        DecimalError::Overflow => overflow("decimal"),
        DecimalError::Syntax => conversion("invalid decimal syntax"),
    }
}

fn ts(t: Option<Timestamp>) -> Result<CelValue, CelError> {
    t.map(CelValue::Timestamp)
        .ok_or_else(|| overflow("timestamp"))
}

fn dur(d: Option<Duration>) -> Result<CelValue, CelError> {
    d.map(CelValue::Duration)
        .ok_or_else(|| overflow("duration"))
}

fn as_decimal(v: &CelValue) -> Option<Decimal> {
    match v {
        CelValue::Decimal(d) => Some(*d),
        CelValue::Int(i) => Some(Decimal::from_i64(*i)),
        CelValue::Uint(u) => Some(Decimal::from_u64(*u)),
        _ => None,
    }
}

fn is_decimal_pair(a: &CelValue, b: &CelValue) -> bool {
    matches!(
        (a, b),
        (
            CelValue::Decimal(_),
            CelValue::Decimal(_) | CelValue::Int(_) | CelValue::Uint(_)
        ) | (CelValue::Int(_) | CelValue::Uint(_), CelValue::Decimal(_))
    )
}

fn charge_len(b: &mut Budget, len: usize) -> Result<(), CelError> {
    b.step(1 + (len as u64) / 64)
}

/// Apply a binary operator other than `&&` and `||`.
pub fn binary(
    op: BinOp,
    a: &CelValue,
    b: &CelValue,
    budget: &mut Budget,
) -> Result<CelValue, CelError> {
    use CelValue as V;
    let name = format!("_{}_", op.symbol());
    let fail = || no_overload(&name, &[a, b]);
    match op {
        BinOp::Eq | BinOp::Ne => {
            let cost = a.deep_size(budget.size_cap()) + b.deep_size(budget.size_cap());
            charge_len(budget, cost)?;
            let eq = a.cel_eq(b);
            Ok(V::Bool(if op == BinOp::Eq { eq } else { !eq }))
        }
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = match (a, b) {
                _ if a.is_number() && b.is_number() => num_cmp(a, b),
                (V::Bool(x), V::Bool(y)) => Some(x.cmp(y)),
                (V::String(x), V::String(y)) => {
                    charge_len(budget, x.len().min(y.len()))?;
                    Some(x.as_bytes().cmp(y.as_bytes()))
                }
                (V::Bytes(x), V::Bytes(y)) => {
                    charge_len(budget, x.len().min(y.len()))?;
                    Some(x.cmp(y))
                }
                (V::Timestamp(x), V::Timestamp(y)) => Some(x.cmp(y)),
                (V::Duration(x), V::Duration(y)) => Some(x.cmp(y)),
                _ => return Err(fail()),
            };
            let r = match ord {
                None => false,
                Some(o) => match op {
                    BinOp::Lt => o == Ordering::Less,
                    BinOp::Le => o != Ordering::Greater,
                    BinOp::Gt => o == Ordering::Greater,
                    _ => o != Ordering::Less,
                },
            };
            Ok(V::Bool(r))
        }
        BinOp::In => match b {
            V::List(l) => {
                let cost = b.deep_size(budget.size_cap());
                charge_len(budget, cost)?;
                Ok(V::Bool(l.iter().any(|x| x.cel_eq(a))))
            }
            V::Map(mp) => {
                let key = match a {
                    V::Double(d) if crate::value::is_integral(*d) && d.abs() < 9.2e18 => {
                        Key::Int(*d as i64)
                    }
                    _ => Key::from_value(a).ok_or_else(fail)?,
                };
                Ok(V::Bool(map_get(mp, &key).is_some()))
            }
            _ => Err(fail()),
        },
        BinOp::Add => match (a, b) {
            (V::Int(x), V::Int(y)) => x.checked_add(*y).map(V::Int).ok_or_else(|| overflow("int")),
            (V::Uint(x), V::Uint(y)) => x
                .checked_add(*y)
                .map(V::Uint)
                .ok_or_else(|| overflow("uint")),
            (V::Double(x), V::Double(y)) => Ok(V::Double(x + y)),
            _ if is_decimal_pair(a, b) => arith_dec(a, b, Decimal::checked_add),
            (V::String(x), V::String(y)) => {
                let n = x.len() + y.len();
                charge_len(budget, n)?;
                budget.check_size(n)?;
                let mut s = String::with_capacity(n);
                s.push_str(x);
                s.push_str(y);
                Ok(V::String(Arc::from(s)))
            }
            (V::Bytes(x), V::Bytes(y)) => {
                let n = x.len() + y.len();
                charge_len(budget, n)?;
                budget.check_size(n)?;
                let mut v = Vec::with_capacity(n);
                v.extend_from_slice(x);
                v.extend_from_slice(y);
                Ok(V::Bytes(Arc::from(v)))
            }
            (V::List(x), V::List(y)) => {
                let n = x.len() + y.len();
                charge_len(budget, n)?;
                budget.check_size(n)?;
                let mut v = Vec::with_capacity(n);
                v.extend(x.iter().cloned());
                v.extend(y.iter().cloned());
                Ok(V::List(Arc::new(v)))
            }
            (V::Timestamp(t), V::Duration(d)) | (V::Duration(d), V::Timestamp(t)) => ts(t.plus(*d)),
            (V::Duration(x), V::Duration(y)) => dur(x.plus(*y)),
            _ => Err(fail()),
        },
        BinOp::Sub => match (a, b) {
            (V::Int(x), V::Int(y)) => x.checked_sub(*y).map(V::Int).ok_or_else(|| overflow("int")),
            (V::Uint(x), V::Uint(y)) => x
                .checked_sub(*y)
                .map(V::Uint)
                .ok_or_else(|| overflow("uint")),
            (V::Double(x), V::Double(y)) => Ok(V::Double(x - y)),
            _ if is_decimal_pair(a, b) => arith_dec(a, b, Decimal::checked_sub),
            (V::Timestamp(x), V::Timestamp(y)) => dur(x.since(*y)),
            (V::Timestamp(t), V::Duration(d)) => {
                ts(Duration::from_nanos(-d.nanos()).and_then(|n| t.plus(n)))
            }
            (V::Duration(x), V::Duration(y)) => dur(Duration::from_nanos(x.nanos() - y.nanos())),
            _ => Err(fail()),
        },
        BinOp::Mul => match (a, b) {
            (V::Int(x), V::Int(y)) => x.checked_mul(*y).map(V::Int).ok_or_else(|| overflow("int")),
            (V::Uint(x), V::Uint(y)) => x
                .checked_mul(*y)
                .map(V::Uint)
                .ok_or_else(|| overflow("uint")),
            (V::Double(x), V::Double(y)) => Ok(V::Double(x * y)),
            _ if is_decimal_pair(a, b) => arith_dec(a, b, Decimal::checked_mul),
            _ => Err(fail()),
        },
        BinOp::Div => match (a, b) {
            (V::Int(_), V::Int(0)) | (V::Uint(_), V::Uint(0)) => {
                Err(CelError::new(ErrorKind::DivisionByZero, "division by zero"))
            }
            (V::Int(x), V::Int(y)) => x.checked_div(*y).map(V::Int).ok_or_else(|| overflow("int")),
            (V::Uint(x), V::Uint(y)) => Ok(V::Uint(x / y)),
            (V::Double(x), V::Double(y)) => Ok(V::Double(x / y)),
            _ if is_decimal_pair(a, b) => arith_dec(a, b, Decimal::checked_div),
            _ => Err(fail()),
        },
        BinOp::Rem => match (a, b) {
            (V::Int(_), V::Int(0)) | (V::Uint(_), V::Uint(0)) => {
                Err(CelError::new(ErrorKind::DivisionByZero, "modulus by zero"))
            }
            (V::Int(x), V::Int(y)) => x.checked_rem(*y).map(V::Int).ok_or_else(|| overflow("int")),
            (V::Uint(x), V::Uint(y)) => Ok(V::Uint(x % y)),
            _ if is_decimal_pair(a, b) => arith_dec(a, b, Decimal::checked_rem),
            _ => Err(fail()),
        },
    }
}

fn arith_dec(
    a: &CelValue,
    b: &CelValue,
    f: fn(Decimal, Decimal) -> Result<Decimal, DecimalError>,
) -> Result<CelValue, CelError> {
    match (as_decimal(a), as_decimal(b)) {
        (Some(x), Some(y)) => f(x, y).map(CelValue::Decimal).map_err(dec_err),
        _ => Err(no_overload("decimal arithmetic", &[a, b])),
    }
}

/// Logical not and negation.
pub fn negate(v: &CelValue) -> Result<CelValue, CelError> {
    match v {
        CelValue::Int(i) => i
            .checked_neg()
            .map(CelValue::Int)
            .ok_or_else(|| overflow("int")),
        CelValue::Double(d) => Ok(CelValue::Double(-d)),
        CelValue::Decimal(d) => Ok(CelValue::Decimal(d.neg())),
        CelValue::Duration(d) => dur(Duration::from_nanos(-d.nanos())),
        _ => Err(no_overload("-_", &[v])),
    }
}

/// Call function `name` (not `In`, which the evaluator answers) on evaluated arguments; for
/// methods `args[0]` is the receiver.
pub fn call(
    name: &str,
    method: bool,
    args: &[CelValue],
    budget: &mut Budget,
) -> Result<CelValue, CelError> {
    use CelValue as V;
    let refs: Vec<&CelValue> = args.iter().collect();
    let fail = || no_overload(name, &refs);
    match (name, method, args) {
        ("size", _, [x]) => {
            let n = match x {
                V::String(s) => {
                    charge_len(budget, s.len())?;
                    s.chars().count()
                }
                V::Bytes(b) => b.len(),
                V::List(l) => l.len(),
                V::Map(m) => m.len(),
                _ => return Err(fail()),
            };
            Ok(V::Int(n as i64))
        }
        ("contains" | "startsWith" | "endsWith", true, [V::String(s), V::String(t)]) => {
            charge_len(budget, s.len())?;
            Ok(V::Bool(match name {
                "contains" => s.contains(&**t),
                "startsWith" => s.starts_with(&**t),
                _ => s.ends_with(&**t),
            }))
        }
        ("matches", _, [V::String(s), V::String(re)]) => {
            budget.step(1 + (s.len() as u64 + 1) * (re.len() as u64 + 1) / 16)?;
            let vm = PikeVM::builder()
                .thompson(thompson::Config::new().nfa_size_limit(Some(1 << 20)))
                .build(re)
                .map_err(|e| {
                    CelError::new(
                        ErrorKind::InvalidArgument,
                        format!("invalid regular expression `{re}`: {e}"),
                    )
                })?;
            let mut cache = vm.create_cache();
            Ok(V::Bool(vm.is_match(&mut cache, s.as_bytes())))
        }
        ("int", false, [x]) => match x {
            V::Int(i) => Ok(V::Int(*i)),
            V::Uint(u) => i64::try_from(*u).map(V::Int).map_err(|_| overflow("int")),
            V::Double(d) => {
                if d.is_finite()
                    && *d >= -9_223_372_036_854_775_808.0
                    && *d < 9_223_372_036_854_775_808.0
                {
                    Ok(V::Int(crate::value::trunc(*d) as i64))
                } else {
                    Err(overflow("int"))
                }
            }
            V::Decimal(d) => d.trunc_i64().map(V::Int).ok_or_else(|| overflow("int")),
            V::String(s) => s
                .parse::<i64>()
                .map(V::Int)
                .map_err(|_| conversion(format!("cannot convert string `{s}` to int"))),
            V::Timestamp(t) => Ok(V::Int(t.secs())),
            _ => Err(fail()),
        },
        ("uint", false, [x]) => match x {
            V::Int(i) => u64::try_from(*i).map(V::Uint).map_err(|_| overflow("uint")),
            V::Uint(u) => Ok(V::Uint(*u)),
            V::Double(d) => {
                if d.is_finite() && *d > -1.0 && *d < 18_446_744_073_709_551_616.0 {
                    Ok(V::Uint(crate::value::trunc(*d) as u64))
                } else {
                    Err(overflow("uint"))
                }
            }
            V::Decimal(d) => d.trunc_u64().map(V::Uint).ok_or_else(|| overflow("uint")),
            V::String(s) => s
                .parse::<u64>()
                .map(V::Uint)
                .map_err(|_| conversion(format!("cannot convert string `{s}` to uint"))),
            _ => Err(fail()),
        },
        ("double", false, [x]) => match x {
            V::Int(i) => Ok(V::Double(*i as f64)),
            V::Uint(u) => Ok(V::Double(*u as f64)),
            V::Double(d) => Ok(V::Double(*d)),
            V::Decimal(d) => Ok(V::Double(d.to_f64())),
            V::String(s) => s
                .parse::<f64>()
                .map(V::Double)
                .map_err(|_| conversion(format!("cannot convert string `{s}` to double"))),
            _ => Err(fail()),
        },
        ("decimal", false, [x]) => match x {
            V::Int(i) => Ok(V::Decimal(Decimal::from_i64(*i))),
            V::Uint(u) => Ok(V::Decimal(Decimal::from_u64(*u))),
            V::Double(d) => Decimal::from_f64(*d)
                .map(V::Decimal)
                .map_err(|_| conversion(format!("cannot convert double {d} to decimal"))),
            V::Decimal(d) => Ok(V::Decimal(*d)),
            V::String(s) => Decimal::parse(s)
                .map(V::Decimal)
                .map_err(|e| conversion(format!("cannot convert string `{s}` to decimal: {e}"))),
            _ => Err(fail()),
        },
        ("string", false, [x]) => {
            let s = match x {
                V::Bool(b) => b.to_string(),
                V::Int(i) => i.to_string(),
                V::Uint(u) => u.to_string(),
                V::Double(d) => format!("{d}"),
                V::Decimal(d) => d.to_string(),
                V::String(s) => return Ok(V::String(s.clone())),
                V::Bytes(b) => match core::str::from_utf8(b) {
                    Ok(s) => s.to_string(),
                    Err(_) => return Err(conversion("bytes are not valid UTF-8")),
                },
                V::Timestamp(t) => t.format(),
                V::Duration(d) => d.format(),
                V::Type(t) => t.to_string(),
                _ => return Err(fail()),
            };
            budget.check_size(s.len())?;
            Ok(V::String(Arc::from(s)))
        }
        ("bytes", false, [x]) => match x {
            V::String(s) => Ok(V::Bytes(Arc::from(s.as_bytes()))),
            V::Bytes(b) => Ok(V::Bytes(b.clone())),
            _ => Err(fail()),
        },
        ("bool", false, [x]) => match x {
            V::Bool(b) => Ok(V::Bool(*b)),
            V::String(s) => match &**s {
                "true" | "True" | "TRUE" | "t" | "1" => Ok(V::Bool(true)),
                "false" | "False" | "FALSE" | "f" | "0" => Ok(V::Bool(false)),
                _ => Err(conversion(format!("cannot convert string `{s}` to bool"))),
            },
            _ => Err(fail()),
        },
        ("timestamp", false, [x]) => match x {
            V::String(s) => Timestamp::parse(s)
                .map(V::Timestamp)
                .ok_or_else(|| conversion(format!("invalid RFC 3339 timestamp `{s}`"))),
            V::Int(i) => ts(Timestamp::from_secs(*i)),
            V::Timestamp(t) => Ok(V::Timestamp(*t)),
            _ => Err(fail()),
        },
        ("duration", false, [x]) => match x {
            V::String(s) => Duration::parse(s)
                .map(V::Duration)
                .ok_or_else(|| conversion(format!("invalid duration `{s}`"))),
            V::Duration(d) => Ok(V::Duration(*d)),
            _ => Err(fail()),
        },
        ("dyn", false, [x]) => Ok(x.clone()),
        ("type", false, [x]) => Ok(V::Type(Arc::from(x.type_name()))),
        (
            "getFullYear" | "getMonth" | "getDate" | "getDayOfMonth" | "getDayOfWeek"
            | "getDayOfYear" | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds",
            true,
            [V::Timestamp(t), rest @ ..],
        ) => {
            let offset = match rest {
                [] => 0,
                [V::String(z)] => parse_zone(z).ok_or_else(|| {
                    CelError::new(
                        ErrorKind::InvalidArgument,
                        format!("unsupported time zone `{z}`: use `UTC` or a fixed offset such as `+02:00`"),
                    )
                })?,
                _ => return Err(fail()),
            };
            let c = t.civil(offset);
            Ok(V::Int(match name {
                "getFullYear" => c.year,
                "getMonth" => c.month - 1,
                "getDate" => c.day,
                "getDayOfMonth" => c.day - 1,
                "getDayOfWeek" => c.weekday,
                "getDayOfYear" => c.yday,
                "getHours" => c.hour,
                "getMinutes" => c.minute,
                "getSeconds" => c.second,
                _ => c.nanos / 1_000_000,
            }))
        }
        ("getHours" | "getMinutes" | "getSeconds" | "getMilliseconds", true, [V::Duration(d)]) => {
            let n = d.nanos();
            let v = match name {
                "getHours" => n / 3_600_000_000_000,
                "getMinutes" => n / 60_000_000_000,
                "getSeconds" => n / 1_000_000_000,
                _ => n % 1_000_000_000 / 1_000_000,
            };
            Ok(V::Int(i64::try_from(v).map_err(|_| overflow("int"))?))
        }
        _ => {
            if lookup(name).is_none() {
                Err(CelError::new(
                    ErrorKind::Undeclared,
                    format!("undeclared reference to function `{name}`"),
                ))
            } else {
                Err(fail())
            }
        }
    }
}
