//! The datamodel answer tape for the Lean oracle (decision D5).
//!
//! [`TapeDatamodel`] wraps any datamodel and records every request the interpreter makes and the
//! answer it got, in order. The Lean `step` treats the datamodel as an oracle: it replays the
//! tape and must issue exactly the same requests in the same order, so the Rust and Lean runs can
//! be compared on charts whose expressions Lean cannot evaluate. `snapshot` takes `&self` and is
//! never called by the interpreter, so it is not recorded; `name` is not recorded either.
//!
//! # Exchange format
//!
//! With the `serde` feature the tape, the trace and the inputs serialize with serde's default
//! (externally tagged) representation. The exchange with the oracle is JSON Lines: one
//! [`OracleRecord`] per line, one line per macrostep, in order:
//!
//! ```text
//! {"input": <OracleInput>, "tape": [<TapeEntry>, ...], "trace": <MacrostepTrace>}
//! ```
//!
//! - `input` is `{"Start": <StartInput>}` for the first line and `{"Step": <Input>}` afterwards.
//! - `tape` lists the datamodel requests of that macrostep as
//!   `{"request": <Request>, "answer": <Answer>}`, for example
//!   `{"request": {"EvalCond": {"expr": "In('b')"}}, "answer": {"Bool": true}}` or
//!   `{"request": {"EvalValue": {"expr": "x"}}, "answer": {"Error": {"message": "..."}}}`.
//!   Unit requests (`SetEvent`, `SetEnvironment`, successful `Declare`, ...) answer `"Unit"`.
//! - `trace` is the [`MacrostepTrace`]: `index`, `input` (`{"Start": {"sessionid": ...}}`,
//!   `{"Event": <Event>}` or `"Cancel"`), `microsteps` (each `{"event": name or null,
//!   "transitions": [{"source": id, "index": n}, ...]}`), `configuration` (state identifiers in
//!   document order), `effects` (each an externally tagged [`crate::session::Effect`]) and
//!   `terminated`.
//!
//! [`Value`]s serialize as `"Undefined"`, `"Null"`, `{"Bool": b}`, `{"Number": n}`,
//! `{"String": s}`, `{"Array": [...]}` and `{"Object": [[key, value], ...]}` (keys in insertion
//! order).

use alloc::string::String;
use alloc::vec::Vec;

use crate::datamodel::{Datamodel, Environment, EvalError, Event, SessionInfo, StateQuery, Value};
use crate::session::{Input, StartInput};
use crate::trace::MacrostepTrace;

/// A datamodel request.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Request {
    /// `init_session`.
    InitSession(SessionInfo),
    /// `set_environment`.
    SetEnvironment(Environment),
    /// `declare`.
    Declare {
        /// Data item id.
        id: String,
    },
    /// `set_event`.
    SetEvent(Event),
    /// `eval_cond`.
    EvalCond {
        /// Expression.
        expr: String,
    },
    /// `eval_value`.
    EvalValue {
        /// Expression.
        expr: String,
    },
    /// `eval_content`.
    EvalContent {
        /// Inline content.
        content: String,
    },
    /// `assign_expr`.
    AssignExpr {
        /// Location.
        location: String,
        /// Expression.
        expr: String,
    },
    /// `assign_value`.
    AssignValue {
        /// Location.
        location: String,
        /// Value.
        value: Value,
    },
    /// `bind_var`.
    BindVar {
        /// Variable name.
        name: String,
        /// Value.
        value: Value,
    },
    /// `is_location`.
    IsLocation {
        /// Location.
        location: String,
    },
    /// `exec_script`.
    ExecScript {
        /// Script text.
        script: String,
    },
    /// `foreach_items`.
    ForeachItems {
        /// Array expression.
        array: String,
    },
    /// `restore`.
    Restore(Value),
}

/// A datamodel answer.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Answer {
    /// Success without a value.
    Unit,
    /// A boolean (conditions, `is_location`).
    Bool(bool),
    /// A value.
    Value(Value),
    /// A list of values (`foreach_items`).
    Values(Vec<Value>),
    /// An evaluation error.
    Error(EvalError),
}

/// One request and its answer.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TapeEntry {
    /// The request.
    pub request: Request,
    /// The answer.
    pub answer: Answer,
}

/// The input of one macrostep in an oracle record.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OracleInput {
    /// The start macrostep.
    Start(StartInput),
    /// A later macrostep.
    Step(Input),
}

/// One JSON Lines record exchanged with the Lean oracle: a macrostep's input, its datamodel
/// tape and its trace.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct OracleRecord {
    /// The input.
    pub input: OracleInput,
    /// Datamodel requests and answers, in order.
    pub tape: Vec<TapeEntry>,
    /// The macrostep trace.
    pub trace: MacrostepTrace,
}

/// A datamodel wrapper that records every request and answer.
pub struct TapeDatamodel<'a> {
    inner: &'a mut dyn Datamodel,
    entries: Vec<TapeEntry>,
}

impl<'a> TapeDatamodel<'a> {
    /// Record the requests made to `inner`.
    pub fn new(inner: &'a mut dyn Datamodel) -> Self {
        TapeDatamodel {
            inner,
            entries: Vec::new(),
        }
    }

    /// The entries recorded so far.
    pub fn entries(&self) -> &[TapeEntry] {
        &self.entries
    }

    /// Take the entries recorded so far, leaving the tape empty (call after each macrostep).
    pub fn take(&mut self) -> Vec<TapeEntry> {
        core::mem::take(&mut self.entries)
    }

    fn log(&mut self, request: Request, answer: Answer) {
        self.entries.push(TapeEntry { request, answer });
    }
}

fn unit(r: &Result<(), EvalError>) -> Answer {
    match r {
        Ok(()) => Answer::Unit,
        Err(e) => Answer::Error(e.clone()),
    }
}

impl Datamodel for TapeDatamodel<'_> {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        let r = self.inner.init_session(info);
        self.log(Request::InitSession(info.clone()), unit(&r));
        r
    }

    fn set_environment(&mut self, env: &Environment) {
        self.inner.set_environment(env);
        self.log(Request::SetEnvironment(*env), Answer::Unit);
    }

    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        let r = self.inner.declare(id);
        self.log(Request::Declare { id: id.into() }, unit(&r));
        r
    }

    fn set_event(&mut self, event: &Event) {
        self.inner.set_event(event);
        self.log(Request::SetEvent(event.clone()), Answer::Unit);
    }

    /// Not an interpreter request (hosts call it before stepping), so it is not recorded.
    fn check_event(&self, event: &Event) -> Result<(), EvalError> {
        self.inner.check_event(event)
    }

    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        let r = self.inner.eval_cond(expr, states);
        let a = match &r {
            Ok(b) => Answer::Bool(*b),
            Err(e) => Answer::Error(e.clone()),
        };
        self.log(Request::EvalCond { expr: expr.into() }, a);
        r
    }

    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError> {
        let r = self.inner.eval_value(expr, states);
        let a = match &r {
            Ok(v) => Answer::Value(v.clone()),
            Err(e) => Answer::Error(e.clone()),
        };
        self.log(Request::EvalValue { expr: expr.into() }, a);
        r
    }

    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        let r = self.inner.eval_content(content);
        let a = match &r {
            Ok(v) => Answer::Value(v.clone()),
            Err(e) => Answer::Error(e.clone()),
        };
        self.log(
            Request::EvalContent {
                content: content.into(),
            },
            a,
        );
        r
    }

    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        let r = self.inner.assign_expr(location, expr, states);
        self.log(
            Request::AssignExpr {
                location: location.into(),
                expr: expr.into(),
            },
            unit(&r),
        );
        r
    }

    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        let req = Request::AssignValue {
            location: location.into(),
            value: value.clone(),
        };
        let r = self.inner.assign_value(location, value);
        self.log(req, unit(&r));
        r
    }

    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        let req = Request::BindVar {
            name: name.into(),
            value: value.clone(),
        };
        let r = self.inner.bind_var(name, value);
        self.log(req, unit(&r));
        r
    }

    fn is_location(&mut self, location: &str) -> bool {
        let r = self.inner.is_location(location);
        self.log(
            Request::IsLocation {
                location: location.into(),
            },
            Answer::Bool(r),
        );
        r
    }

    fn exec_script(&mut self, script: &str, states: &dyn StateQuery) -> Result<(), EvalError> {
        let r = self.inner.exec_script(script, states);
        self.log(
            Request::ExecScript {
                script: script.into(),
            },
            unit(&r),
        );
        r
    }

    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        let r = self.inner.foreach_items(array, states);
        let a = match &r {
            Ok(v) => Answer::Values(v.clone()),
            Err(e) => Answer::Error(e.clone()),
        };
        self.log(
            Request::ForeachItems {
                array: array.into(),
            },
            a,
        );
        r
    }

    fn snapshot(&self) -> Value {
        self.inner.snapshot()
    }

    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        let r = self.inner.restore(snapshot);
        self.log(Request::Restore(snapshot.clone()), unit(&r));
        r
    }
}
