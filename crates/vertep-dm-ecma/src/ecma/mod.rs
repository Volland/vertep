//! The W3C SCXML ECMAScript datamodel (Appendix B.2) on the boa engine.
//!
//! One boa [`Context`] per session holds the global scope. Data items are global variables,
//! `<script>` runs as a global script, and the read-only system variables are accessor
//! properties of the global object whose setters throw.
//!
//! Chart text is never spliced into code before boa's parser has confirmed that it is exactly
//! one expression or one location (see `syntax`). Value expressions run in a sloppy function in
//! the global scope; assignments run in a strict function, so assigning to an undeclared name
//! or a read-only property is an error; conditions run inside `with` over a read-only view.

mod convert;
mod syntax;
#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use boa_engine::context::time::{Clock, JsInstant};
use boa_engine::context::{ContextBuilder, HostHooks};
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::PropertyDescriptor;
use boa_engine::{
    Context, JsError, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, Source,
    js_string,
};
use vertep_core::datamodel::{
    Datamodel, Environment, EvalError, Event, EventType, SessionInfo, StateQuery, Value,
};

use self::convert::{from_js, to_js};
use self::syntax::Location;

/// The datamodel name used in the `datamodel` attribute.
pub const NAME: &str = "ecmascript";

/// Version tag of the snapshot format produced by [`EcmaDatamodel`].
pub const SNAPSHOT_FORMAT: &str = "vertep-ecma/1";

/// The system variables of W3C SCXML 5.10, read-only in every datamodel.
const SYSTEM_VARIABLES: [&str; 4] = ["_event", "_sessionid", "_name", "_ioprocessors"];

/// Default number of iterations any single loop may run before evaluation fails, so a runaway
/// script ends in an evaluation error instead of hanging the session. Deterministic, unlike a
/// time budget.
pub const DEFAULT_LOOP_ITERATION_LIMIT: u64 = 10_000_000;

/// Global name used only while restoring a snapshot, to hand values to lexical declarations.
const RESTORE_SLOT: &str = "__vertepRestoreSlot__";

const RUNTIME_JS: &str = include_str!("runtime.js");

/// Host-side state the native functions read through the context.
#[derive(Debug, Default)]
struct HostState {
    /// SplitMix64 state behind `Math.random`.
    rng: Cell<u64>,
    /// The active states of the evaluation in progress, when the caller listed them.
    active: RefCell<Option<BTreeSet<String>>>,
    /// Answers to `In(id)` for prefetched candidate ids, used when the caller cannot list the
    /// active states.
    in_answers: RefCell<BTreeMap<String, bool>>,
}

/// The clock behind `Date`, `Date.now()` and boa's monotonic time: the input's timestamp.
#[derive(Debug, Default)]
struct InputClock {
    now_ms: Cell<i64>,
}

impl Clock for InputClock {
    fn now(&self) -> JsInstant {
        let ms = u64::try_from(self.now_ms.get()).unwrap_or(0);
        // `ms % 1000 * 1_000_000` is below 10^9 and fits in a u32.
        let nanos = u32::try_from(ms % 1000 * 1_000_000).unwrap_or(0);
        JsInstant::new(ms / 1000, nanos)
    }

    fn system_time_millis(&self) -> i64 {
        self.now_ms.get()
    }
}

/// Host hooks that pin the local time zone to UTC, so local-time `Date` methods do not depend
/// on the machine the session runs on.
#[derive(Debug)]
struct DeterministicHooks;

impl HostHooks for DeterministicHooks {
    fn local_timezone_offset_seconds(&self, _unix_time_seconds: i64) -> i32 {
        0
    }
}

fn splitmix_next(state: &Cell<u64>) -> u64 {
    let next = state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
    state.set(next);
    let mut z = next;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn host_state(ctx: &Context) -> JsResult<&HostState> {
    ctx.get_data::<HostState>().ok_or_else(|| {
        JsNativeError::typ()
            .with_message("datamodel host state is missing")
            .into()
    })
}

/// `Math.random`, seeded from [`Environment::seed`].
fn math_random(_this: &JsValue, _args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let bits = splitmix_next(&host_state(ctx)?.rng) >> 11;
    // 53 random bits scaled into [0, 1); both conversions are exact.
    #[allow(clippy::cast_precision_loss)]
    let value = bits as f64 / (1u64 << 53) as f64;
    Ok(JsValue::rational(value))
}

/// `In(stateId)`, answered from the [`StateQuery`] of the evaluation in progress.
fn in_predicate(_this: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let id = args
        .first()
        .cloned()
        .unwrap_or_default()
        .to_string(ctx)?
        .to_std_string_escaped();
    let host = host_state(ctx)?;
    let answer = match host.active.borrow().as_ref() {
        Some(active) => Some(active.contains(&id)),
        None => host.in_answers.borrow().get(&id).copied(),
    };
    match answer {
        Some(active) => Ok(JsValue::new(active)),
        None => Err(JsNativeError::typ()
            .with_message(format!(
                "In('{id}'): the caller cannot list its active states, so the state id must appear as a string literal in a chart expression or script"
            ))
            .into()),
    }
}

/// Turns an engine error into an [`EvalError`], without the engine's backtrace.
fn eval_error(err: &JsError, ctx: &mut Context) -> EvalError {
    let text = match err.try_native(ctx) {
        Ok(native) => native.to_string(),
        Err(_) => err.to_string(),
    };
    EvalError::new(text.split("\n    at ").next().unwrap_or_default())
}

/// Equality of snapshots that treats NaN as equal to itself.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
        }
        (Value::Array(xs), Value::Array(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| same_value(x, y))
        }
        (Value::Object(xs), Value::Object(ys)) => {
            xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys)
                    .all(|((kx, x), (ky, y))| kx == ky && same_value(x, y))
        }
        _ => a == b,
    }
}

fn event_type_name(type_: EventType) -> &'static str {
    match type_ {
        EventType::Platform => "platform",
        EventType::Internal => "internal",
        EventType::External => "external",
    }
}

fn opt_string(value: Option<&String>) -> Value {
    value.map_or(Value::Undefined, |s| Value::String(s.clone()))
}

/// Items of the last `foreach` arrays, kept so that `bind_var` can bind the very objects of the
/// array (a shallow copy) rather than deep copies rebuilt from their [`Value`]s.
#[derive(Debug, Default)]
struct PendingItems {
    items: Vec<(Value, JsValue)>,
    cursor: usize,
}

/// How many `foreach` arrays (nested loops) keep their items for identity-preserving binding.
const PENDING_DEPTH: usize = 8;

/// One session's ECMAScript state.
struct Session {
    // Engine handles come first so they are dropped before the context.
    runtime: JsObject,
    value_fns: HashMap<String, JsObject>,
    guard_fns: HashMap<String, JsObject>,
    assign_fns: HashMap<String, JsObject>,
    read_fns: HashMap<String, JsObject>,
    pending: Vec<PendingItems>,
    clock: Rc<InputClock>,
    info: SessionInfo,
    event: Option<Event>,
    env: Environment,
    declared: BTreeSet<String>,
    lexical: Vec<(String, bool)>,
    in_candidates: BTreeSet<String>,
    ctx: Context,
}

impl Session {
    fn new(loop_limit: u64) -> Result<Self, EvalError> {
        let clock = Rc::new(InputClock::default());
        let mut ctx = ContextBuilder::new()
            .host_hooks(Rc::new(DeterministicHooks))
            .clock(clock.clone())
            .build()
            .map_err(|e| EvalError::new(format!("cannot start the ECMAScript engine: {e}")))?;
        ctx.insert_data(HostState::default());
        ctx.runtime_limits_mut()
            .set_loop_iteration_limit(loop_limit);
        let runtime = Self::install(&mut ctx).map_err(|e| eval_error(&e, &mut ctx))?;
        Ok(Session {
            runtime,
            value_fns: HashMap::new(),
            guard_fns: HashMap::new(),
            assign_fns: HashMap::new(),
            read_fns: HashMap::new(),
            pending: Vec::new(),
            clock,
            info: SessionInfo::default(),
            event: None,
            env: Environment::default(),
            declared: BTreeSet::new(),
            lexical: Vec::new(),
            in_candidates: BTreeSet::new(),
            ctx,
        })
    }

    /// Installs `Math.random`, `In` and the runtime; returns the runtime's helper object.
    fn install(ctx: &mut Context) -> JsResult<JsObject> {
        let global = ctx.global_object();
        let math = global
            .get(js_string!("Math"), ctx)?
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("Math is missing"))?;
        let random =
            FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(math_random))
                .name(js_string!("random"))
                .length(0)
                .build();
        math.define_property_or_throw(
            js_string!("random"),
            PropertyDescriptor::builder()
                .value(random)
                .writable(true)
                .enumerable(false)
                .configurable(true),
            ctx,
        )?;
        let in_fn =
            FunctionObjectBuilder::new(ctx.realm(), NativeFunction::from_fn_ptr(in_predicate))
                .name(js_string!("In"))
                .length(1)
                .build();
        global.define_property_or_throw(
            js_string!("In"),
            PropertyDescriptor::builder()
                .value(in_fn)
                .writable(false)
                .enumerable(false)
                .configurable(false),
            ctx,
        )?;
        let factory = ctx.eval(Source::from_bytes(RUNTIME_JS))?;
        let factory = factory
            .as_callable()
            .ok_or_else(|| JsNativeError::typ().with_message("runtime is not a function"))?;
        let runtime = factory
            .call(&JsValue::undefined(), &[global.into()], ctx)?
            .as_object()
            .ok_or_else(|| JsNativeError::typ().with_message("runtime returned a non-object"))?;
        let seal = runtime
            .get(js_string!("sealBaseline"), ctx)?
            .as_callable()
            .ok_or_else(|| JsNativeError::typ().with_message("runtime has no sealBaseline"))?;
        seal.call(&JsValue::undefined(), &[], ctx)?;
        Ok(runtime)
    }

    fn helper(&mut self, name: &str) -> JsResult<JsObject> {
        self.runtime
            .get(JsString::from(name), &mut self.ctx)?
            .as_callable()
            .ok_or_else(|| {
                JsNativeError::typ()
                    .with_message(format!("runtime helper {name} is missing"))
                    .into()
            })
    }

    fn call_helper(&mut self, name: &str, args: &[JsValue]) -> JsResult<JsValue> {
        let f = self.helper(name)?;
        f.call(&JsValue::undefined(), args, &mut self.ctx)
    }

    fn err(&mut self, e: &JsError) -> EvalError {
        eval_error(e, &mut self.ctx)
    }

    fn value_of(&mut self, value: &JsValue) -> JsResult<Value> {
        let keys = self.helper("keys")?;
        let is_array = self.helper("isArray")?;
        from_js(value, &keys, &is_array, &mut self.ctx)
    }

    fn compile(&mut self, code: &str) -> JsResult<JsObject> {
        self.ctx
            .eval(Source::from_bytes(code))?
            .as_object()
            .ok_or_else(|| {
                JsNativeError::typ()
                    .with_message("compiled code is not a function")
                    .into()
            })
    }

    /// A function returning the value of `expr`; `expr` must already be checked.
    fn value_fn(&mut self, expr: &str) -> JsResult<JsObject> {
        if let Some(f) = self.value_fns.get(expr) {
            return Ok(f.clone());
        }
        let f = self.compile(&format!("(function () {{ return ({expr}\n); }})"))?;
        self.value_fns.insert(expr.to_owned(), f.clone());
        Ok(f)
    }

    /// A function evaluating `expr` inside `with` over its scope argument.
    fn guard_fn(&mut self, expr: &str) -> JsResult<JsObject> {
        if let Some(f) = self.guard_fns.get(expr) {
            return Ok(f.clone());
        }
        let f = self.compile(&format!(
            "(function (__vertepScope__) {{ with (__vertepScope__) {{ return ({expr}\n); }} }})"
        ))?;
        self.guard_fns.insert(expr.to_owned(), f.clone());
        Ok(f)
    }

    /// A strict function assigning its argument to `location`, which must already be checked.
    fn assign_fn(&mut self, location: &str) -> JsResult<JsObject> {
        if let Some(f) = self.assign_fns.get(location) {
            return Ok(f.clone());
        }
        let f = self.compile(&format!(
            "(function (__vertepValue__) {{ \"use strict\"; ({location}\n) = __vertepValue__; }})"
        ))?;
        self.assign_fns.insert(location.to_owned(), f.clone());
        Ok(f)
    }

    /// A strict function reading `location`, which must already be checked.
    fn read_fn(&mut self, location: &str) -> JsResult<JsObject> {
        if let Some(f) = self.read_fns.get(location) {
            return Ok(f.clone());
        }
        let f = self.compile(&format!(
            "(function () {{ \"use strict\"; return ({location}\n); }})"
        ))?;
        self.read_fns.insert(location.to_owned(), f.clone());
        Ok(f)
    }

    fn note_candidates(&mut self, text: &str) {
        self.in_candidates.extend(syntax::in_candidates(text));
    }

    /// Answers `In()` for every candidate state id against `states`.
    /// Copies what `In()` needs out of `states`: the whole active configuration when the
    /// caller can list it, otherwise answers for every candidate id seen as a literal.
    fn prepare_in(&mut self, text: &str, states: &dyn StateQuery) {
        self.note_candidates(text);
        let active: Option<BTreeSet<String>> =
            states.active_states().map(|ids| ids.into_iter().collect());
        let answers = if active.is_some() {
            BTreeMap::new()
        } else {
            self.in_candidates
                .iter()
                .map(|id| (id.clone(), states.in_state(id)))
                .collect()
        };
        if let Some(host) = self.ctx.get_data::<HostState>() {
            *host.active.borrow_mut() = active;
            *host.in_answers.borrow_mut() = answers;
        }
    }

    fn clear_in(&mut self) {
        if let Some(host) = self.ctx.get_data::<HostState>() {
            host.active.borrow_mut().take();
            host.in_answers.borrow_mut().clear();
        }
    }

    fn rng(&self) -> u64 {
        self.ctx.get_data::<HostState>().map_or(0, |h| h.rng.get())
    }

    fn set_rng(&mut self, value: u64) {
        if let Some(host) = self.ctx.get_data::<HostState>() {
            host.rng.set(value);
        }
    }

    fn set_sys(&mut self, name: &str, value: JsValue) -> JsResult<()> {
        self.call_helper("setSys", &[JsString::from(name).into(), value])?;
        Ok(())
    }

    fn run_jobs(&mut self) -> JsResult<()> {
        self.ctx.run_jobs()
    }

    // ---- Datamodel operations -----------------------------------------------------------

    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        let result = (|| {
            self.set_sys("_sessionid", JsString::from(info.sessionid.as_str()).into())?;
            let name = info
                .name
                .as_deref()
                .map_or(JsValue::undefined(), |n| JsString::from(n).into());
            self.set_sys("_name", name)?;
            let io = to_js(&info.ioprocessors, &mut self.ctx)?;
            self.set_sys("_ioprocessors", io)?;
            Ok(())
        })();
        result.map_err(|e: JsError| self.err(&e))?;
        self.info = info.clone();
        Ok(())
    }

    fn set_environment(&mut self, env: &Environment) {
        self.clock.now_ms.set(env.now_ms);
        self.set_rng(env.seed);
        self.env = *env;
    }

    fn set_event(&mut self, event: &Event) -> Result<(), EvalError> {
        self.pending.clear();
        let fields = Value::Object(vec![
            ("name".into(), Value::String(event.name.clone())),
            (
                "type".into(),
                Value::String(event_type_name(event.type_).into()),
            ),
            ("sendid".into(), opt_string(event.sendid.as_ref())),
            ("origin".into(), opt_string(event.origin.as_ref())),
            ("origintype".into(), opt_string(event.origintype.as_ref())),
            ("invokeid".into(), opt_string(event.invokeid.as_ref())),
            ("data".into(), event.data.clone()),
        ]);
        let result = to_js(&fields, &mut self.ctx).and_then(|v| self.set_sys("_event", v));
        result.map_err(|e| self.err(&e))?;
        self.event = Some(event.clone());
        Ok(())
    }

    fn check_not_system(name: &str) -> Result<(), EvalError> {
        if SYSTEM_VARIABLES.contains(&name) {
            Err(EvalError::new(format!(
                "TypeError: cannot assign to the read-only system variable {name}"
            )))
        } else {
            Ok(())
        }
    }

    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        let name = syntax::parse_variable(id)?;
        Self::check_not_system(&name)?;
        if self.lexical.iter().any(|(n, _)| *n == name) {
            return Err(EvalError::new(format!(
                "cannot declare {name}: a script already declared it with let, const or class"
            )));
        }
        let declared = self
            .call_helper("declare", &[JsString::from(name.as_str()).into()])
            .map_err(|e| self.err(&e))?;
        if !declared.to_boolean() {
            return Err(EvalError::new(format!("cannot declare {name}")));
        }
        self.declared.insert(name);
        Ok(())
    }

    fn exists(&mut self, name: &str) -> Result<bool, EvalError> {
        self.call_helper("exists", &[JsString::from(name).into()])
            .map(|v| v.to_boolean())
            .map_err(|e| self.err(&e))
    }

    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        syntax::check_expression(expr)?;
        self.prepare_in(expr, states);
        let rng = self.rng();
        let outcome = (|| {
            let keys_before = self.call_helper("globalKeys", &[])?;
            let result = (|| {
                let f = self.guard_fn(expr)?;
                let scope = self.call_helper("guardScope", &[])?;
                f.call(&scope, std::slice::from_ref(&scope), &mut self.ctx)
            })();
            let removed = self.call_helper("removeNewKeys", &[keys_before])?;
            Ok::<_, JsError>((result, removed.to_boolean()))
        })();
        self.set_rng(rng);
        self.clear_in();
        let (result, created_globals) = outcome.map_err(|e| self.err(&e))?;
        let value = result.map_err(|e| self.err(&e))?;
        if created_globals {
            return Err(EvalError::new(
                "TypeError: a condition must not modify the datamodel",
            ));
        }
        Ok(value.to_boolean())
    }

    /// Evaluates a checked expression to an engine value.
    fn eval_js(&mut self, expr: &str, states: &dyn StateQuery) -> Result<JsValue, EvalError> {
        syntax::check_expression(expr)?;
        self.prepare_in(expr, states);
        let result = (|| {
            let f = self.value_fn(expr)?;
            let value = f.call(&JsValue::undefined(), &[], &mut self.ctx)?;
            self.run_jobs()?;
            Ok(value)
        })();
        self.clear_in();
        result.map_err(|e: JsError| self.err(&e))
    }

    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError> {
        let value = self.eval_js(expr, states)?;
        self.value_of(&value).map_err(|e| self.err(&e))
    }

    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            let parsed = self.call_helper("jsonParse", &[JsString::from(trimmed).into()]);
            if let Ok(parsed) = parsed {
                return self.value_of(&parsed).map_err(|e| self.err(&e));
            }
        }
        let normalized: Vec<&str> = content
            .split([' ', '\t', '\n', '\r'])
            .filter(|part| !part.is_empty())
            .collect();
        Ok(Value::String(normalized.join(" ")))
    }

    /// Assigns an engine value to an existing location.
    fn assign_js(&mut self, location: &str, value: JsValue) -> Result<(), EvalError> {
        let parsed = syntax::parse_location(location)?;
        match &parsed {
            Location::Variable(name) | Location::Member { root: Some(name) } => {
                Self::check_not_system(name)?;
            }
            Location::Member { root: None } => {}
        }
        let result = (|| {
            let f = self.assign_fn(location)?;
            f.call(&JsValue::undefined(), &[value], &mut self.ctx)?;
            self.run_jobs()
        })();
        result.map_err(|e| self.err(&e))
    }

    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        syntax::parse_location(location)?;
        let value = self.eval_js(expr, states)?;
        self.assign_js(location, value)
    }

    fn assign_value(&mut self, location: &str, value: &Value) -> Result<(), EvalError> {
        let value = to_js(value, &mut self.ctx).map_err(|e| self.err(&e))?;
        self.assign_js(location, value)
    }

    /// The engine value for `value`: the original array item when `value` came from
    /// [`Session::foreach_items`], otherwise a fresh conversion.
    fn take_pending(&mut self, value: &Value) -> Option<JsValue> {
        if !matches!(value, Value::Array(_) | Value::Object(_)) {
            return None;
        }
        for list in self.pending.iter_mut().rev() {
            let start = list.cursor;
            let found = list.items[start..]
                .iter()
                .position(|(v, js)| js.is_object() && same_value(v, value));
            if let Some(offset) = found {
                list.cursor = start + offset + 1;
                return Some(list.items[start + offset].1.clone());
            }
        }
        None
    }

    fn bind_var(&mut self, name: &str, value: &Value) -> Result<(), EvalError> {
        if let Location::Variable(variable) = syntax::parse_location(name)?
            && !self.exists(&variable)?
        {
            self.declare(&variable)?;
        }
        let js = match self.take_pending(value) {
            Some(js) => js,
            None => to_js(value, &mut self.ctx).map_err(|e| self.err(&e))?,
        };
        self.assign_js(name, js)
    }

    fn is_location(&mut self, location: &str) -> bool {
        let Ok(parsed) = syntax::parse_location(location) else {
            return false;
        };
        match &parsed {
            Location::Variable(name) | Location::Member { root: Some(name) } => {
                if SYSTEM_VARIABLES.contains(&name.as_str()) {
                    return false;
                }
            }
            Location::Member { root: None } => {}
        }
        // Compiling the assignment proves the location is assignable in strict code; reading it
        // proves that the variable and every object on the path exist.
        if self.assign_fn(location).is_err() {
            return false;
        }
        self.read_fn(location)
            .and_then(|f| f.call(&JsValue::undefined(), &[], &mut self.ctx))
            .is_ok()
    }

    fn exec_script(&mut self, script: &str, states: &dyn StateQuery) -> Result<(), EvalError> {
        self.prepare_in(script, states);
        let result = self
            .ctx
            .eval(Source::from_bytes(script))
            .and_then(|_| self.run_jobs());
        self.clear_in();
        if script.contains("let") || script.contains("const") || script.contains("class") {
            let bindings = syntax::script_bindings(script);
            for (name, is_const) in bindings.lexical {
                self.register_lexical(name, is_const);
            }
            for (name, source) in bindings.classes {
                let args = [
                    JsString::from(name.as_str()).into(),
                    JsString::from(source.as_str()).into(),
                ];
                // A class whose binding failed to initialize has nothing to register.
                let _ = self.call_helper("registerClass", &args);
            }
        }
        result.map_err(|e| self.err(&e))
    }

    /// Records a global lexical binding once it holds a value.
    fn register_lexical(&mut self, name: String, is_const: bool) {
        if self.lexical.iter().any(|(n, _)| *n == name) {
            return;
        }
        if self.ctx.eval(Source::from_bytes(name.as_str())).is_err() {
            return;
        }
        if self
            .call_helper("addLexical", &[JsString::from(name.as_str()).into()])
            .is_ok()
        {
            self.lexical.push((name, is_const));
            // boa binds a function's free names to global lexical bindings when it compiles
            // the function, so code compiled before this binding existed would not see it.
            self.value_fns.clear();
            self.guard_fns.clear();
            self.assign_fns.clear();
            self.read_fns.clear();
        }
    }

    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        let collection = self.eval_js(array, states)?;
        let result = (|| {
            let is_array = self.call_helper("isArray", std::slice::from_ref(&collection))?;
            let object = match collection.as_object() {
                Some(object) if is_array.to_boolean() => object,
                _ => {
                    return Err(JsNativeError::typ()
                        .with_message(format!("foreach: `{array}` is not an array"))
                        .into());
                }
            };
            let length = object.get(js_string!("length"), &mut self.ctx)?;
            let length = u32::try_from(length.to_length(&mut self.ctx)?).unwrap_or(u32::MAX);
            let mut items = Vec::new();
            for index in 0..length {
                let item = object.get(index, &mut self.ctx)?;
                // An item that has no `Value` form still binds by identity.
                let value = self
                    .value_of(&item)
                    .unwrap_or_else(|_| Value::Object(Vec::new()));
                items.push((value, item));
            }
            Ok(items)
        })();
        let items = result.map_err(|e: JsError| self.err(&e))?;
        let values = items.iter().map(|(v, _)| v.clone()).collect();
        if self.pending.len() == PENDING_DEPTH {
            self.pending.remove(0);
        }
        self.pending.push(PendingItems { items, cursor: 0 });
        Ok(values)
    }

    // ---- Snapshot ----------------------------------------------------------------------

    fn snapshot(&mut self) -> Value {
        match self.encode() {
            Ok(value) => value,
            Err(e) => {
                let message = self.err(&e).message;
                Value::Object(vec![
                    ("format".into(), Value::String(SNAPSHOT_FORMAT.into())),
                    ("error".into(), Value::String(message)),
                ])
            }
        }
    }

    fn encode(&mut self) -> JsResult<Value> {
        let declared: Vec<Value> = self
            .declared
            .iter()
            .map(|n| Value::String(n.clone()))
            .collect();
        let declared_js = to_js(&Value::Array(declared.clone()), &mut self.ctx)?;
        let names = self.call_helper("userGlobalNames", &[declared_js])?;
        let lexical = Value::Array(
            self.lexical
                .iter()
                .map(|(n, c)| Value::Array(vec![Value::String(n.clone()), Value::Bool(*c)]))
                .collect(),
        );
        let lexical_js = to_js(&lexical, &mut self.ctx)?;
        let state = self.call_helper("encodeState", &[names, lexical_js])?;
        let Value::Object(state) = self.value_of(&state)? else {
            return Err(JsNativeError::typ()
                .with_message("snapshot encoder returned a non-object")
                .into());
        };
        let event = self.event.as_ref().map_or(Value::Undefined, |e| {
            Value::Object(vec![
                ("name".into(), Value::String(e.name.clone())),
                (
                    "type".into(),
                    Value::String(event_type_name(e.type_).into()),
                ),
                ("sendid".into(), opt_string(e.sendid.as_ref())),
                ("origin".into(), opt_string(e.origin.as_ref())),
                ("origintype".into(), opt_string(e.origintype.as_ref())),
                ("invokeid".into(), opt_string(e.invokeid.as_ref())),
                ("data".into(), e.data.clone()),
            ])
        });
        let mut out = vec![
            ("format".into(), Value::String(SNAPSHOT_FORMAT.into())),
            (
                "session".into(),
                Value::Object(vec![
                    (
                        "sessionid".into(),
                        Value::String(self.info.sessionid.clone()),
                    ),
                    ("name".into(), opt_string(self.info.name.as_ref())),
                    ("ioprocessors".into(), self.info.ioprocessors.clone()),
                ]),
            ),
            ("event".into(), event),
            (
                "env".into(),
                Value::Object(vec![
                    ("now_ms".into(), Value::String(self.env.now_ms.to_string())),
                    ("seed".into(), Value::String(self.env.seed.to_string())),
                ]),
            ),
            ("rng".into(), Value::String(self.rng().to_string())),
            ("declared".into(), Value::Array(declared)),
            (
                "in_candidates".into(),
                Value::Array(
                    self.in_candidates
                        .iter()
                        .map(|c| Value::String(c.clone()))
                        .collect(),
                ),
            ),
        ];
        out.extend(state);
        Ok(Value::Object(out))
    }

    /// Rebuilds the state described by `snapshot` into this fresh session.
    fn decode(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        let snap = Snapshot::new(snapshot)?;
        if let Some(Value::String(error)) = snap.field("error") {
            return Err(EvalError::new(format!(
                "snapshot could not be taken: {error}"
            )));
        }
        let session = snap.object("session")?;
        let info = SessionInfo {
            sessionid: session.string("sessionid")?,
            name: session.opt_string("name")?,
            ioprocessors: session.field("ioprocessors").cloned().unwrap_or_default(),
        };
        self.init_session(&info)?;
        let env = snap.object("env")?;
        let env = Environment {
            now_ms: env.parsed("now_ms")?,
            seed: env.parsed("seed")?,
        };
        self.set_environment(&env);
        let rng = snap.parsed("rng")?;
        self.set_rng(rng);
        if let Some(event) = snap.field("event")
            && !matches!(event, Value::Undefined)
        {
            let event = Snapshot::new(event)?;
            let type_ = match event.string("type")?.as_str() {
                "platform" => EventType::Platform,
                "internal" => EventType::Internal,
                "external" => EventType::External,
                other => {
                    return Err(EvalError::new(format!(
                        "snapshot: unknown event type {other}"
                    )));
                }
            };
            self.set_event(&Event {
                name: event.string("name")?,
                type_,
                sendid: event.opt_string("sendid")?,
                origin: event.opt_string("origin")?,
                origintype: event.opt_string("origintype")?,
                invokeid: event.opt_string("invokeid")?,
                data: event.field("data").cloned().unwrap_or_default(),
            })?;
        }
        for name in snap.strings("declared")? {
            self.declared.insert(name);
        }
        self.in_candidates.extend(snap.strings("in_candidates")?);
        let heap = snap
            .field("heap")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        let globals = snap
            .field("globals")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        // Every named binding, so that class heritage can be resolved while decoding.
        let mut bindings = Vec::new();
        for list in [snap.field("globals"), snap.field("lexical")]
            .into_iter()
            .flatten()
        {
            if let Value::Array(entries) = list {
                for entry in entries {
                    if let Value::Array(parts) = entry
                        && let [name, encoded, ..] = parts.as_slice()
                    {
                        bindings.push(Value::Array(vec![name.clone(), encoded.clone()]));
                    }
                }
            }
        }
        let mut lexical = Vec::new();
        if let Some(Value::Array(entries)) = snap.field("lexical") {
            for entry in entries {
                let Value::Array(parts) = entry else {
                    return Err(EvalError::new("snapshot: malformed lexical binding"));
                };
                let [Value::String(name), encoded, Value::Bool(is_const)] = parts.as_slice() else {
                    return Err(EvalError::new("snapshot: malformed lexical binding"));
                };
                lexical.push((syntax::parse_variable(name)?, encoded.clone(), *is_const));
            }
        }
        let encoded = Value::Array(lexical.iter().map(|(_, e, _)| e.clone()).collect());
        let result = (|| {
            let heap = to_js(&heap, &mut self.ctx)?;
            let bindings = to_js(&Value::Array(bindings), &mut self.ctx)?;
            let globals = to_js(&globals, &mut self.ctx)?;
            let encoded = to_js(&encoded, &mut self.ctx)?;
            self.call_helper("prepareDecode", &[heap, bindings, globals, encoded])?;
            if !lexical.is_empty() {
                self.declare_lexicals(&lexical)?;
            }
            self.call_helper("endDecode", &[])?;
            Ok(())
        })();
        result.map_err(|e: JsError| self.err(&e))?;
        for (name, _, is_const) in lexical {
            self.lexical.push((name, is_const));
        }
        Ok(())
    }

    /// Declares the restored global lexical bindings in one script (see `prepareDecode`).
    fn declare_lexicals(&mut self, lexical: &[(String, Value, bool)]) -> JsResult<()> {
        let slot = self.helper("restoreSlot")?;
        let global = self.ctx.global_object();
        global.define_property_or_throw(
            JsString::from(RESTORE_SLOT),
            PropertyDescriptor::builder()
                .value(slot)
                .writable(true)
                .enumerable(false)
                .configurable(true),
            &mut self.ctx,
        )?;
        let mut script = String::new();
        for (index, (name, _, is_const)) in lexical.iter().enumerate() {
            let keyword = if *is_const { "const" } else { "let" };
            script.push_str(&format!("{keyword} {name} = {RESTORE_SLOT}({index});\n"));
        }
        let declared = self.ctx.eval(Source::from_bytes(&script));
        global.delete_property_or_throw(JsString::from(RESTORE_SLOT), &mut self.ctx)?;
        declared?;
        for (name, _, _) in lexical {
            self.call_helper("addLexical", &[JsString::from(name.as_str()).into()])?;
        }
        Ok(())
    }
}

/// Field access on a snapshot object.
struct Snapshot<'a> {
    fields: &'a [(String, Value)],
}

impl<'a> Snapshot<'a> {
    fn new(value: &'a Value) -> Result<Self, EvalError> {
        match value {
            Value::Object(fields) => Ok(Snapshot { fields }),
            _ => Err(EvalError::new("snapshot: expected an object")),
        }
    }

    fn field(&self, key: &str) -> Option<&'a Value> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn object(&self, key: &str) -> Result<Snapshot<'a>, EvalError> {
        self.field(key)
            .ok_or_else(|| EvalError::new(format!("snapshot: missing {key}")))
            .and_then(Snapshot::new)
    }

    fn string(&self, key: &str) -> Result<String, EvalError> {
        match self.field(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            _ => Err(EvalError::new(format!("snapshot: {key} must be a string"))),
        }
    }

    fn opt_string(&self, key: &str) -> Result<Option<String>, EvalError> {
        match self.field(key) {
            None | Some(Value::Undefined) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            _ => Err(EvalError::new(format!("snapshot: {key} must be a string"))),
        }
    }

    fn parsed<T: std::str::FromStr>(&self, key: &str) -> Result<T, EvalError> {
        self.string(key)?
            .parse()
            .map_err(|_| EvalError::new(format!("snapshot: {key} is not a number")))
    }

    fn strings(&self, key: &str) -> Result<Vec<String>, EvalError> {
        match self.field(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    Value::String(s) => Ok(s.clone()),
                    _ => Err(EvalError::new(format!("snapshot: {key} must hold strings"))),
                })
                .collect(),
            Some(_) => Err(EvalError::new(format!("snapshot: {key} must be an array"))),
        }
    }
}

/// The W3C SCXML ECMAScript datamodel (Appendix B.2) on the boa engine.
///
/// Each instance owns one engine context, which is the global scope of one session. Conditions
/// evaluate against a read-only view of that scope; `Math.random` and `Date` read only the
/// host-supplied [`Environment`]; [`Datamodel::snapshot`] captures the whole scope, including
/// functions defined by scripts, as a datamodel-independent [`Value`].
// @tg: implements:: [[openspec:datamodel#ECMAScript Datamodel]]
pub struct EcmaDatamodel {
    // `snapshot` takes `&self` but reading engine objects needs `&mut Context`.
    session: RefCell<Session>,
    loop_limit: u64,
}

impl core::fmt::Debug for EcmaDatamodel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EcmaDatamodel").finish_non_exhaustive()
    }
}

impl EcmaDatamodel {
    /// A fresh datamodel with an empty global scope.
    ///
    /// # Errors
    ///
    /// Fails only if the bundled runtime cannot start, which is a bug.
    pub fn try_new() -> Result<Self, EvalError> {
        Ok(EcmaDatamodel {
            session: RefCell::new(Session::new(DEFAULT_LOOP_ITERATION_LIMIT)?),
            loop_limit: DEFAULT_LOOP_ITERATION_LIMIT,
        })
    }

    /// Sets how many iterations any single loop may run before evaluation fails (default
    /// [`DEFAULT_LOOP_ITERATION_LIMIT`]). The limit is part of the host configuration, not of
    /// the snapshot, and is kept across [`Datamodel::restore`].
    pub fn set_loop_iteration_limit(&mut self, limit: u64) {
        self.loop_limit = limit;
        self.session()
            .ctx
            .runtime_limits_mut()
            .set_loop_iteration_limit(limit);
    }

    /// A fresh datamodel with an empty global scope.
    ///
    /// # Panics
    ///
    /// Panics only if the bundled runtime cannot start, which is a bug; see [`Self::try_new`].
    #[must_use]
    pub fn new() -> Self {
        match Self::try_new() {
            Ok(dm) => dm,
            Err(e) => panic!("the ECMAScript datamodel runtime failed to start: {e}"),
        }
    }

    fn session(&mut self) -> &mut Session {
        self.session.get_mut()
    }

    /// Debug-build backstop for guard purity: the datamodel after a condition must equal the
    /// datamodel before it; if not, the state is rolled back and the condition fails.
    #[cfg(debug_assertions)]
    fn checked_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        self.session().note_candidates(expr);
        let before = self.snapshot();
        let result = self.session().eval_cond(expr, states);
        let after = self.snapshot();
        if same_value(&before, &after) {
            return result;
        }
        self.restore(&before)?;
        Err(EvalError::new(format!(
            "TypeError: condition `{expr}` modified the datamodel; the change was rolled back"
        )))
    }
}

impl Default for EcmaDatamodel {
    fn default() -> Self {
        Self::new()
    }
}

impl Datamodel for EcmaDatamodel {
    fn name(&self) -> &str {
        NAME
    }

    // @tg: implements:: [[openspec:datamodel#System Variables]]
    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        self.session().init_session(info)
    }

    // @tg: implements:: [[openspec:datamodel#Deterministic Evaluation]]
    fn set_environment(&mut self, env: &Environment) {
        self.session().set_environment(env);
    }

    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        self.session().declare(id)
    }

    fn set_event(&mut self, event: &Event) {
        // Building `_event` fails only if the engine is out of resources; the previous event
        // then stays visible, which the next operation's error will surface.
        let _ = self.session().set_event(event);
    }

    // @tg: implements:: [[openspec:datamodel#Guard Purity]]
    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        self.session().pending.clear();
        #[cfg(debug_assertions)]
        {
            self.checked_cond(expr, states)
        }
        #[cfg(not(debug_assertions))]
        {
            self.session().eval_cond(expr, states)
        }
    }

    fn eval_value(&mut self, expr: &str, states: &dyn StateQuery) -> Result<Value, EvalError> {
        self.session().eval_value(expr, states)
    }

    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        self.session().eval_content(content)
    }

    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        self.session().assign_expr(location, expr, states)
    }

    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        self.session().assign_value(location, &value)
    }

    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        self.session().bind_var(name, &value)
    }

    fn is_location(&mut self, location: &str) -> bool {
        self.session().is_location(location)
    }

    fn exec_script(&mut self, script: &str, states: &dyn StateQuery) -> Result<(), EvalError> {
        self.session().exec_script(script, states)
    }

    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        self.session().foreach_items(array, states)
    }

    // @tg: implements:: [[openspec:datamodel#Datamodel Snapshot]]
    fn snapshot(&self) -> Value {
        match self.session.try_borrow_mut() {
            Ok(mut session) => session.snapshot(),
            Err(_) => Value::Object(vec![
                ("format".into(), Value::String(SNAPSHOT_FORMAT.into())),
                ("error".into(), Value::String("datamodel is busy".into())),
            ]),
        }
    }

    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        let mut fresh = Session::new(self.loop_limit)?;
        fresh.decode(snapshot)?;
        *self.session.get_mut() = fresh;
        Ok(())
    }
}
