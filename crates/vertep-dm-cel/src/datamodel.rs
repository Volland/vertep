//! [`CelDatamodel`]: the `cel` datamodel behind the shared [`Datamodel`] contract.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use vertep_core::datamodel::{
    Datamodel, Environment, EvalError, Event, EventType, SessionInfo, StateQuery, Value,
};

use crate::check::{CheckScope, check};
use crate::convert::{
    self, Location, RESERVED_NAMES, Seg, coerce, location_type, parse_location, put,
};
use crate::error::{CelError, ErrorKind, Limits};
use crate::eval::{Budget, Scope, evaluate};
use crate::json;
use crate::syntax::{Expr, parse};
use crate::types::{CelType, TypeEnv};
use crate::value::{CelMap, CelValue, Key, from_value, to_value};

/// The datamodel name used in `datamodel="cel"` and `datamodel: cel`.
pub const CEL: &str = "cel";

/// The `format` field of a [`CelDatamodel::snapshot`].
pub const SNAPSHOT_FORMAT: &str = "vertep-dm-cel/1";

/// The CEL datamodel.
///
/// Data items are the fields of the context map `ctx`; a bare location `x` means `ctx.x`.
/// Expressions see `ctx`, `ev` (also `_event`), `session`, `_sessionid`, `_name`,
/// `_ioprocessors` and `In(id)`, and can change nothing: the only mutations are
/// [`Datamodel::assign_expr`], [`Datamodel::assign_value`] and [`Datamodel::bind_var`].
///
/// How the SCXML executable content maps onto expression-only CEL
/// ([[openspec:cel-datamodel#SCXML Datamodel Contract]]):
///
/// - `<data id expr>` and `<assign location expr>` store the expression's value at the
///   location; with a typed context the value is converted to the location's declared type
///   or the assignment fails and leaves the location unchanged.
/// - `<data>`, `<assign>` and `<content>` bodies are JSON when they parse as JSON, and
///   otherwise a string with runs of whitespace collapsed and trimmed.
/// - `<foreach array item index>` iterates a list in index order or a map's keys in key
///   order; `item` and `index` are context fields, created when missing in an untyped chart.
/// - `<if cond>`, transition `cond` are conditions of type `bool`; `<log expr>`, `<param>`,
///   `<donedata>` and `<content expr>` evaluate to values that leave the datamodel in the
///   canonical value encoding.
/// - `<script>` does not exist in CEL: loading rejects it and executing it is an error.
// @tg: implements:: [[openspec:cel-datamodel#SCXML Datamodel Contract]]
// @tg: implements:: [[openspec:cel-datamodel#CEL Evaluation Scope]]
#[derive(Clone, Debug)]
pub struct CelDatamodel {
    pub(crate) env: Arc<TypeEnv>,
    pub(crate) limits: Limits,
    ctx: CelValue,
    event: CelValue,
    session: CelValue,
    sessionid: CelValue,
    name: CelValue,
    ioprocessors: CelValue,
    pub(crate) programs: BTreeMap<String, Arc<Expr>>,
    pub(crate) loci: BTreeMap<String, Vec<String>>,
    last: Option<(Value, CelValue)>,
    foreach: Vec<Frame>,
    last_steps: u64,
}

/// Exact CEL values handed out by `foreach_items`, matched back when the interpreter binds
/// them, because [`Value`] cannot distinguish `int` from `double` or carry a `decimal`.
#[derive(Clone, Debug)]
struct Frame {
    item: Option<String>,
    items: Vec<(Value, CelValue)>,
    cursor: usize,
}

impl Default for CelDatamodel {
    fn default() -> Self {
        CelDatamodel::new(TypeEnv::untyped(), Limits::default())
    }
}

struct NoStates;

impl StateQuery for NoStates {
    fn in_state(&self, _id: &str) -> bool {
        false
    }
}

impl CelDatamodel {
    /// A CEL datamodel over `env` with `limits`. Expressions are parsed and checked the first
    /// time they are used; [`crate::load`] does it for a whole chart up front.
    // @tg: implements:: [[openspec:cel-datamodel#CEL Datamodel Selection]]
    pub fn new(env: TypeEnv, limits: Limits) -> Self {
        CelDatamodel {
            env: Arc::new(env),
            limits,
            ctx: CelValue::Map(Arc::new(CelMap::new())),
            event: CelValue::Null,
            session: CelValue::Null,
            sessionid: CelValue::str(""),
            name: CelValue::Null,
            ioprocessors: CelValue::Null,
            programs: BTreeMap::new(),
            loci: BTreeMap::new(),
            last: None,
            foreach: Vec::new(),
            last_steps: 0,
        }
    }

    /// The type environment.
    pub fn env(&self) -> &TypeEnv {
        &self.env
    }

    /// The limits.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The context `ctx` (a map from data item names to values).
    pub fn context(&self) -> &CelValue {
        &self.ctx
    }

    /// Steps used by the most recent evaluation.
    pub fn last_steps(&self) -> u64 {
        self.last_steps
    }

    /// Evaluate `src` to an exact CEL value (without the canonical-encoding conversion of
    /// [`Datamodel::eval_value`]); `states` answers `In()`.
    pub fn evaluate(&mut self, src: &str, states: &dyn StateQuery) -> Result<CelValue, CelError> {
        self.run(src, states)
    }

    fn program(&mut self, src: &str) -> Result<Arc<Expr>, CelError> {
        if let Some(p) = self.programs.get(src) {
            return Ok(p.clone());
        }
        if src.len() > self.limits.max_expr_len {
            return Err(CelError::new(
                ErrorKind::SizeLimit,
                format!("expression longer than {} bytes", self.limits.max_expr_len),
            ));
        }
        let e = parse(src, self.limits.max_depth).map_err(|e| {
            CelError::new(
                ErrorKind::Syntax,
                format!("syntax error at offset {}: {}", e.pos, e.message),
            )
        })?;
        let scope = CheckScope {
            env: &self.env,
            event_data: CelType::Dyn,
        };
        if let Err(errs) = check(&e, &scope) {
            let msgs: Vec<String> = errs.into_iter().map(|e| e.message).collect();
            let kind = if msgs.iter().all(|m| m.starts_with("undeclared")) {
                ErrorKind::Undeclared
            } else {
                ErrorKind::Type
            };
            return Err(CelError::new(kind, msgs.join("; ")));
        }
        let p = Arc::new(e);
        self.programs.insert(String::from(src), p.clone());
        Ok(p)
    }

    fn scope<'a>(&'a self, states: &'a dyn StateQuery) -> Scope<'a> {
        Scope {
            ctx: &self.ctx,
            ev: &self.event,
            session: &self.session,
            sessionid: &self.sessionid,
            name: &self.name,
            ioprocessors: &self.ioprocessors,
            states,
        }
    }

    fn run(&mut self, src: &str, states: &dyn StateQuery) -> Result<CelValue, CelError> {
        self.last_steps = 0;
        let prog = self.program(src)?;
        let mut budget = Budget::new(self.limits);
        let r = evaluate(&prog, &self.scope(states), &mut budget).and_then(|v| {
            budget.check_value(&v)?;
            Ok(v)
        });
        self.last_steps = budget.used();
        r
    }

    /// The `EvalError` for a failure of `src`, naming the expression, the places in the chart
    /// where it occurs and the failure ([[openspec:cel-datamodel#Runtime Errors Raise Error Execution]]).
    // @tg: implements:: [[openspec:cel-datamodel#Runtime Errors Raise Error Execution]]
    pub fn describe_error(&self, src: &str, e: &CelError) -> EvalError {
        let at = match self.loci.get(src) {
            Some(l) if !l.is_empty() => format!(" at {}", l.join("; ")),
            _ => String::new(),
        };
        EvalError::new(format!("cel: `{src}`{at}: {}", e.message))
    }

    fn fail(&self, src: &str, e: CelError) -> EvalError {
        self.describe_error(src, &e)
    }

    fn located(&self, loc: &str, e: CelError) -> EvalError {
        EvalError::new(format!("cel: location `{loc}`: {}", e.message))
    }

    /// Resolve a location string: the fast path for a declared item whose name is not a CEL
    /// identifier (a data id such as `my-id`), else the location grammar.
    fn location(&self, src: &str) -> Result<Location, CelError> {
        if let CelValue::Map(m) = &self.ctx
            && m.contains_key(&Key::str(src))
        {
            return Ok(Location {
                root: Arc::from(src),
                path: Vec::new(),
            });
        }
        parse_location(src, self.limits.max_depth)
    }

    /// The value at a bare location (`Var1`, `Var1.a`) whose root is a data item, or `None`.
    ///
    /// The interpreter reads `namelist` names and `<param location>` with
    /// [`Datamodel::eval_value`], because the datamodel contract has no location read; [`crate::load`]
    /// checks them as locations, where a bare location means a field of `ctx`. So an expression
    /// that fails only as an undeclared reference and is such a location reads the location
    /// (IRP tests 240, 241, 244, 245, 354 under CEL).
    fn read_location(&mut self, src: &str, states: &dyn StateQuery) -> Option<CelValue> {
        let loc = self.location(src.trim()).ok()?;
        let CelValue::Map(m) = &self.ctx else {
            return None;
        };
        let mut cur = m.get(&Key::String(loc.root.clone()))?.clone();
        let keys = self.keys(&loc, states).ok()?;
        for k in keys.iter().skip(1) {
            cur = crate::eval::index_value(&cur, k).ok()?;
        }
        Some(cur)
    }

    /// Evaluate the index expressions of a location's path into keys.
    fn keys(&mut self, loc: &Location, states: &dyn StateQuery) -> Result<Vec<CelValue>, CelError> {
        let mut keys = Vec::with_capacity(loc.path.len() + 1);
        keys.push(CelValue::String(loc.root.clone()));
        for s in &loc.path {
            keys.push(match s {
                Seg::Field(f) => CelValue::String(f.clone()),
                Seg::Index(e) => {
                    let mut budget = Budget::new(self.limits);
                    evaluate(e, &self.scope(states), &mut budget)?
                }
            });
        }
        Ok(keys)
    }

    /// Store `value` at `loc`, converted to the location's declared type; on any failure the
    /// context is unchanged.
    fn store(
        &mut self,
        src: &str,
        loc: &Location,
        value: CelValue,
        states: &dyn StateQuery,
    ) -> Result<(), CelError> {
        let declared =
            matches!(&self.ctx, CelValue::Map(m) if m.contains_key(&Key::String(loc.root.clone())));
        if !declared {
            return Err(CelError::new(
                ErrorKind::Location,
                format!("`{src}` is not a declared data item"),
            ));
        }
        let ty =
            location_type(&self.env, loc).map_err(|m| CelError::new(ErrorKind::Location, m))?;
        let value = coerce(value, &ty, src)?;
        Budget::new(self.limits).check_value(&value)?;
        let keys = self.keys(loc, states)?;
        let mut next = self.ctx.clone();
        put(&mut next, &keys, value)?;
        self.ctx = next;
        Ok(())
    }

    fn take_exact(&mut self, value: &Value) -> CelValue {
        match self.last.take() {
            Some((v, c)) if v == *value => c,
            _ => from_value(value),
        }
    }

    fn remember(&mut self, cel: CelValue) -> Value {
        let v = to_value(&cel);
        self.last = Some((v.clone(), cel));
        v
    }

    /// Match a `bind_var` against the items of the innermost `foreach` still running.
    fn foreach_exact(&mut self, name: &str, value: &Value) -> Option<CelValue> {
        while self
            .foreach
            .last()
            .is_some_and(|f| f.cursor >= f.items.len())
        {
            self.foreach.pop();
        }
        for f in self.foreach.iter_mut().rev() {
            let named = match &f.item {
                None => true,
                Some(n) => n == name,
            };
            if named && f.items.get(f.cursor).is_some_and(|(v, _)| v == value) {
                f.item = Some(String::from(name));
                let c = f.items[f.cursor].1.clone();
                f.cursor += 1;
                return Some(c);
            }
        }
        None
    }

    fn event_value(&self, e: &Event) -> CelValue {
        let opt = |s: &Option<String>| s.as_deref().map_or(CelValue::Null, CelValue::str);
        let mut data = from_value(&e.data);
        if let Some(t) = self.env.event_payload(&e.name)
            && let Ok(d) = coerce(data.clone(), t, "ev.data")
        {
            data = d;
        }
        let ty = match e.type_ {
            EventType::Platform => "platform",
            EventType::Internal => "internal",
            EventType::External => "external",
        };
        let mut m = CelMap::new();
        m.insert(Key::str("name"), CelValue::str(&e.name));
        m.insert(Key::str("type"), CelValue::str(ty));
        m.insert(Key::str("sendid"), opt(&e.sendid));
        m.insert(Key::str("origin"), opt(&e.origin));
        m.insert(Key::str("origintype"), opt(&e.origintype));
        m.insert(Key::str("invokeid"), opt(&e.invokeid));
        m.insert(Key::str("data"), data);
        CelValue::Map(Arc::new(m))
    }
}

/// Collapse runs of XML whitespace to one space and trim.
fn normalize_space(s: &str) -> String {
    s.split([' ', '\t', '\n', '\r'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Datamodel for CelDatamodel {
    fn name(&self) -> &str {
        CEL
    }

    // @tg: implements:: [[openspec:datamodel#System Variables]]
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        self.sessionid = CelValue::str(&info.sessionid);
        self.name = info.name.as_deref().map_or(CelValue::Null, CelValue::str);
        self.ioprocessors = from_value(&info.ioprocessors);
        let mut m = CelMap::new();
        m.insert(Key::str("id"), self.sessionid.clone());
        m.insert(Key::str("name"), self.name.clone());
        self.session = CelValue::Map(Arc::new(m));
        Ok(())
    }

    /// CEL reads neither a clock nor a random source, so the environment is unused.
    fn set_environment(&mut self, _env: &Environment) {}

    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        if RESERVED_NAMES.contains(&id) || id.is_empty() {
            return Err(EvalError::new(format!(
                "cel: `{id}` cannot be a data item: the name is reserved"
            )));
        }
        if let Err(m) = self.env.context_field(id) {
            return Err(EvalError::new(format!("cel: data item `{id}`: {m}")));
        }
        if let CelValue::Map(m) = &mut self.ctx {
            Arc::make_mut(m).insert(Key::str(id), CelValue::Null);
        }
        Ok(())
    }

    fn set_event(&mut self, event: &Event) {
        self.event = self.event_value(event);
        self.foreach.clear();
        self.last = None;
    }

    // @tg: implements:: [[openspec:datamodel#Guard Purity]]
    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        self.last = None;
        match self.run(expr, states) {
            Ok(CelValue::Bool(b)) => Ok(b),
            Ok(other) => Err(self.fail(
                expr,
                CelError::new(
                    ErrorKind::Type,
                    format!("a guard must have type bool, found {}", other.type_name()),
                ),
            )),
            Err(e) => Err(self.fail(expr, e)),
        }
    }

    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError> {
        self.last = None;
        match self.run(expr, states) {
            Ok(v) => Ok(self.remember(v)),
            Err(e) if e.kind == ErrorKind::Undeclared => match self.read_location(expr, states) {
                Some(v) => Ok(self.remember(v)),
                None => Err(self.fail(expr, e)),
            },
            Err(e) => Err(self.fail(expr, e)),
        }
    }

    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        let v = json::parse(content, self.limits.max_depth)
            .unwrap_or_else(|| CelValue::String(Arc::from(normalize_space(content))));
        Ok(self.remember(v))
    }

    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        self.last = None;
        let loc = self
            .location(location)
            .map_err(|e| self.located(location, e))?;
        let v = self.run(expr, states).map_err(|e| self.fail(expr, e))?;
        self.store(location, &loc, v, states)
            .map_err(|e| self.located(location, e))
    }

    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        let cel = self.take_exact(&value);
        let loc = self
            .location(location)
            .map_err(|e| self.located(location, e))?;
        self.store(location, &loc, cel, &NoStates)
            .map_err(|e| self.located(location, e))
    }

    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        let cel = self
            .foreach_exact(name, &value)
            .unwrap_or_else(|| from_value(&value));
        let loc = self.location(name).map_err(|e| self.located(name, e))?;
        let declared =
            matches!(&self.ctx, CelValue::Map(m) if m.contains_key(&Key::String(loc.root.clone())));
        if !declared && loc.path.is_empty() {
            self.declare(&loc.root)?;
        }
        self.store(name, &loc, cel, &NoStates)
            .map_err(|e| self.located(name, e))
    }

    fn is_location(&mut self, location: &str) -> bool {
        let Ok(loc) = self.location(location) else {
            return false;
        };
        let CelValue::Map(m) = &self.ctx else {
            return false;
        };
        let Some(mut cur) = m.get(&Key::String(loc.root.clone())).cloned() else {
            return false;
        };
        let Ok(keys) = self.keys(&loc, &NoStates) else {
            return false;
        };
        // Every step but the last must exist; the last may name a new map key.
        let n = keys.len();
        for (i, k) in keys.iter().enumerate().skip(1) {
            match crate::eval::index_value(&cur, k) {
                Ok(v) => cur = v,
                Err(_) => return i == n - 1 && matches!(cur, CelValue::Map(_)),
            }
        }
        true
    }

    /// CEL is expression-only: there are no scripts.
    fn exec_script(&mut self, _script: &str, _states: &dyn StateQuery) -> Result<(), EvalError> {
        Err(EvalError::new(
            "cel: <script> is not supported: CEL is expression-only; use <assign>",
        ))
    }

    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        self.last = None;
        let v = self.run(array, states).map_err(|e| self.fail(array, e))?;
        let items: Vec<CelValue> = match &v {
            CelValue::List(l) => l.iter().cloned().collect(),
            CelValue::Map(m) => m.keys().map(Key::to_value).collect(),
            other => {
                return Err(self.fail(
                    array,
                    CelError::new(
                        ErrorKind::NoMatchingOverload,
                        format!("foreach needs a list or a map, found {}", other.type_name()),
                    ),
                ));
            }
        };
        let pairs: Vec<(Value, CelValue)> = items.into_iter().map(|c| (to_value(&c), c)).collect();
        let out = pairs.iter().map(|(v, _)| v.clone()).collect();
        if self.foreach.len() < 64 {
            self.foreach.push(Frame {
                item: None,
                items: pairs,
                cursor: 0,
            });
        }
        Ok(out)
    }

    // @tg: implements:: [[openspec:datamodel#Datamodel Snapshot]]
    fn snapshot(&self) -> Value {
        let s = |k: &str, v: Value| (String::from(k), v);
        Value::Object(Vec::from([
            s("format", Value::String(String::from(SNAPSHOT_FORMAT))),
            s("ctx", convert::encode(&self.ctx)),
            s("event", convert::encode(&self.event)),
            s("session", convert::encode(&self.session)),
            s("sessionid", convert::encode(&self.sessionid)),
            s("name", convert::encode(&self.name)),
            s("ioprocessors", convert::encode(&self.ioprocessors)),
        ]))
    }

    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        let Value::Object(fields) = snapshot else {
            return Err(EvalError::new("cel: snapshot is not an object"));
        };
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
        if get("format") != Some(&Value::String(String::from(SNAPSHOT_FORMAT))) {
            return Err(EvalError::new(format!(
                "cel: snapshot format is not {SNAPSHOT_FORMAT}"
            )));
        }
        let dec = |k: &str| -> Result<CelValue, EvalError> {
            let v = get(k).ok_or_else(|| EvalError::new(format!("cel: snapshot lacks `{k}`")))?;
            convert::decode(v).map_err(|m| EvalError::new(format!("cel: snapshot `{k}`: {m}")))
        };
        let ctx = dec("ctx")?;
        if !matches!(ctx, CelValue::Map(_)) {
            return Err(EvalError::new("cel: snapshot `ctx` is not a map"));
        }
        let (event, session) = (dec("event")?, dec("session")?);
        let (sessionid, name, io) = (dec("sessionid")?, dec("name")?, dec("ioprocessors")?);
        self.ctx = ctx;
        self.event = event;
        self.session = session;
        self.sessionid = sessionid;
        self.name = name;
        self.ioprocessors = io;
        self.foreach.clear();
        self.last = None;
        Ok(())
    }
}

impl CelDatamodel {
    /// The context value at `location`, for hosts and tests.
    pub fn get(&mut self, location: &str) -> Result<CelValue, CelError> {
        let loc = self.location(location)?;
        let keys = self.keys(&loc, &NoStates)?;
        let mut cur = self.ctx.clone();
        for k in &keys {
            cur = crate::eval::index_value(&cur, k)?;
        }
        Ok(cur)
    }
}
