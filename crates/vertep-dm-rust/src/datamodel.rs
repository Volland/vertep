//! The `rust` datamodel: context as a Rust struct, guards and actions as registered Rust
//! functions.

use std::sync::Arc;

use vertep_core::datamodel::{
    Datamodel, DatamodelRegistry, Environment, EvalError, Event, SessionInfo, StateQuery, Value,
};
use vertep_core::null::parse_in_predicate;
use vertep_core::types::{BasicTyper, ExprError, ExprType, ExprTyper, Scope, Type};

use crate::decimal::Decimal;
use crate::emit::Sys;
use crate::lowering;
use crate::registry::{Registry, RegistryError};

/// The name of the `rust` datamodel.
pub const RUST_DATAMODEL: &str = "rust";

/// The context of a chart on the `rust` datamodel: one field per `<data>` item. The macros
/// generate it as a struct (`Ctx`); [`DynContext`] holds any fields as values.
pub trait Context: Default + Clone + 'static {
    /// Accept the data item `field`: it must be a field of the context.
    fn declare(&mut self, field: &str) -> Result<(), EvalError>;

    /// The value of `field` in its canonical encoding.
    fn get(&self, field: &str) -> Option<Value>;

    /// Set `field` from a value, which must have the field's type.
    fn set(&mut self, field: &str, value: &Value) -> Result<(), EvalError>;

    /// The whole context as a record value.
    fn snapshot(&self) -> Value;

    /// A context from [`Context::snapshot`].
    fn restore(value: &Value) -> Result<Self, EvalError>;
}

/// A context of untyped fields, for charts loaded at runtime without generated types.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DynContext {
    /// Fields in declaration order.
    pub fields: Vec<(String, Value)>,
}

impl Context for DynContext {
    fn declare(&mut self, field: &str) -> Result<(), EvalError> {
        if !self.fields.iter().any(|(k, _)| k == field) {
            self.fields.push((field.to_string(), Value::Undefined));
        }
        Ok(())
    }

    fn get(&self, field: &str) -> Option<Value> {
        self.fields
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v.clone())
    }

    fn set(&mut self, field: &str, value: &Value) -> Result<(), EvalError> {
        match self.fields.iter_mut().find(|(k, _)| k == field) {
            Some((_, v)) => {
                *v = value.clone();
                Ok(())
            }
            None => Err(EvalError::new(format!("unknown context field `{field}`"))),
        }
    }

    fn snapshot(&self) -> Value {
        Value::Object(self.fields.clone())
    }

    fn restore(value: &Value) -> Result<Self, EvalError> {
        match value {
            Value::Object(fields) => Ok(DynContext {
                fields: fields.clone(),
            }),
            _ => Err(EvalError::new("a context snapshot is an object")),
        }
    }
}

/// The `rust` datamodel over the context `C`.
///
/// Guards are called with `&C` and the typed event; actions with `&mut C`, the typed event and
/// an [`crate::Emit`]. Every other expression is a literal or a path into the context, the
/// current event or a loop variable (see [`crate::lowering`]). Evaluation reads no clock and
/// no randomness: the input's time and seed reach actions only through [`Sys`].
// @tg: implements:: [[openspec:rust-dsl#Rust Datamodel]]
pub struct RustDatamodel<C> {
    registry: Arc<Registry<C>>,
    ctx: C,
    vars: Vec<(String, Value)>,
    sys: Sys,
}

impl<C: Context> RustDatamodel<C> {
    /// A fresh datamodel calling the functions in `registry`.
    pub fn new(registry: Arc<Registry<C>>) -> Self {
        RustDatamodel {
            registry,
            ctx: C::default(),
            vars: Vec::new(),
            sys: Sys::default(),
        }
    }

    /// A fresh datamodel for `chart`, failing when a guard or action IRI the chart names has
    /// no registered function.
    pub fn for_chart(
        chart: &vertep_core::ir::Chart,
        registry: Arc<Registry<C>>,
    ) -> Result<Self, Vec<RegistryError>> {
        registry.check_chart(chart)?;
        Ok(RustDatamodel::new(registry))
    }

    /// The context.
    pub fn context(&self) -> &C {
        &self.ctx
    }

    /// The system variables as actions see them.
    pub fn sys(&self) -> &Sys {
        &self.sys
    }

    /// The registry.
    pub fn registry(&self) -> &Arc<Registry<C>> {
        &self.registry
    }

    fn var(&self, name: &str) -> Option<&Value> {
        self.vars.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    fn run_action(&mut self, iri: &str) -> Result<Vec<Value>, EvalError> {
        let action = self
            .registry
            .resolve_action(iri)
            .map_err(|e| EvalError::new(e.to_string()))?
            .clone();
        // The action works on a copy, so a failing payload conversion leaves the context as it
        // was.
        let mut ctx = self.ctx.clone();
        let emitted = action(&mut ctx, &self.sys.event, &self.sys)?;
        self.ctx = ctx;
        Ok(emitted.iter().map(|e| e.to_value()).collect())
    }

    fn call_guard(&self, iri: &str) -> Result<bool, EvalError> {
        let guard = self
            .registry
            .resolve_guard(iri)
            .map_err(|e| EvalError::new(e.to_string()))?;
        guard(&self.ctx, &self.sys.event)
    }

    /// Evaluate a path: a loop variable, a system variable or a context field, then fields.
    fn path(&self, expr: &str) -> Option<Result<Value, EvalError>> {
        let segs: Vec<&str> = expr.split('.').collect();
        if segs.iter().any(|s| {
            s.is_empty()
                || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || s.starts_with(|c: char| c.is_ascii_digit())
        }) {
            return None;
        }
        let (root, rest): (Value, &[&str]) = match segs[0] {
            "_event" => (event_value(&self.sys.event), &segs[1..]),
            "_sessionid" => (Value::String(self.sys.info.sessionid.clone()), &segs[1..]),
            "_name" => (
                self.sys
                    .info
                    .name
                    .clone()
                    .map_or(Value::Null, Value::String),
                &segs[1..],
            ),
            "_ioprocessors" => (self.sys.info.ioprocessors.clone(), &segs[1..]),
            "ctx" if segs.len() >= 2 => match self.ctx.get(segs[1]) {
                Some(v) => (v, &segs[2..]),
                None => {
                    return Some(Err(EvalError::new(format!(
                        "unknown context field `{}`",
                        segs[1]
                    ))));
                }
            },
            name => match self.var(name) {
                Some(v) => (v.clone(), &segs[1..]),
                None => match self.ctx.get(name) {
                    Some(v) => (v, &segs[1..]),
                    None => {
                        return Some(Err(EvalError::new(format!("unknown name `{name}`"))));
                    }
                },
            },
        };
        let mut v = root;
        for s in rest {
            v = match v {
                Value::Object(entries) => entries
                    .into_iter()
                    .find(|(k, _)| k == s)
                    .map(|(_, v)| v)
                    .unwrap_or(Value::Undefined),
                _ => return Some(Err(EvalError::new(format!("no field `{s}` in `{expr}`")))),
            };
        }
        Some(Ok(v))
    }
}

fn event_value(e: &Event) -> Value {
    let opt = |o: &Option<String>| o.clone().map_or(Value::Undefined, Value::String);
    let ty = match e.type_ {
        vertep_core::datamodel::EventType::Platform => "platform",
        vertep_core::datamodel::EventType::Internal => "internal",
        vertep_core::datamodel::EventType::External => "external",
    };
    Value::Object(vec![
        ("name".to_string(), Value::String(e.name.clone())),
        ("type".to_string(), Value::String(ty.to_string())),
        ("sendid".to_string(), opt(&e.sendid)),
        ("origin".to_string(), opt(&e.origin)),
        ("origintype".to_string(), opt(&e.origintype)),
        ("invokeid".to_string(), opt(&e.invokeid)),
        ("data".to_string(), e.data.clone()),
    ])
}

/// Set `path` (field segments) inside `target` to `value`.
fn set_in(target: &mut Value, path: &[&str], value: Value) -> Result<(), EvalError> {
    let Some((first, rest)) = path.split_first() else {
        *target = value;
        return Ok(());
    };
    match target {
        Value::Object(entries) => {
            if let Some((_, v)) = entries.iter_mut().find(|(k, _)| k == first) {
                set_in(v, rest, value)
            } else if rest.is_empty() {
                entries.push((first.to_string(), value));
                Ok(())
            } else {
                Err(EvalError::new(format!("no field `{first}`")))
            }
        }
        _ => Err(EvalError::new(format!(
            "cannot set field `{first}` of a non-record"
        ))),
    }
}

impl<C: Context> Datamodel for RustDatamodel<C> {
    fn name(&self) -> &str {
        RUST_DATAMODEL
    }

    fn init_session(&mut self, info: &SessionInfo) -> Result<(), EvalError> {
        self.sys.info = info.clone();
        Ok(())
    }

    fn set_environment(&mut self, env: &Environment) {
        self.sys.env = *env;
    }

    fn declare(&mut self, id: &str) -> Result<(), EvalError> {
        self.ctx.declare(id)
    }

    fn set_event(&mut self, event: &Event) {
        self.sys.event = event.clone();
    }

    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        let e = expr.trim();
        if let Some(inner) = e.strip_prefix('!') {
            return self.eval_cond(inner, states).map(|b| !b);
        }
        if let Some(id) = parse_in_predicate(e) {
            return Ok(states.in_state(id));
        }
        match e {
            "true" => return Ok(true),
            "false" => return Ok(false),
            _ => {}
        }
        if lowering::is_iri(e) {
            return self.call_guard(e);
        }
        match self.eval_value(e, states)? {
            Value::Bool(b) => Ok(b),
            _ => Err(EvalError::new(format!("condition `{e}` is not a bool"))),
        }
    }

    fn eval_value(&mut self, expr: &str, _states: &dyn StateQuery) -> Result<Value, EvalError> {
        let e = expr.trim();
        if let Some(v) = parse_literal(e) {
            return Ok(v);
        }
        match self.path(e) {
            Some(r) => r,
            None => Err(EvalError::new(format!(
                "`{e}` is not a literal or a path; the rust datamodel evaluates logic in Rust \
                 guards and actions"
            ))),
        }
    }

    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        Ok(parse_literal(content.trim()).unwrap_or_else(|| Value::String(content.to_string())))
    }

    fn assign_expr(
        &mut self,
        location: &str,
        expr: &str,
        states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        let v = self.eval_value(expr, states)?;
        self.assign_value(location, v)
    }

    fn assign_value(&mut self, location: &str, value: Value) -> Result<(), EvalError> {
        let loc = location.trim();
        let loc = loc.strip_prefix("ctx.").unwrap_or(loc);
        let segs: Vec<&str> = loc.split('.').collect();
        let root = segs[0];
        if let Some((_, v)) = self.vars.iter_mut().find(|(k, _)| k == root) {
            return set_in(v, &segs[1..], value);
        }
        let Some(mut current) = self.ctx.get(root) else {
            return Err(EvalError::new(format!("`{location}` is not a location")));
        };
        set_in(&mut current, &segs[1..], value)?;
        self.ctx.set(root, &current)
    }

    fn bind_var(&mut self, name: &str, value: Value) -> Result<(), EvalError> {
        match self.vars.iter_mut().find(|(k, _)| k == name) {
            Some((_, v)) => *v = value,
            None => self.vars.push((name.to_string(), value)),
        }
        Ok(())
    }

    fn is_location(&mut self, location: &str) -> bool {
        let loc = location.trim();
        let loc = loc.strip_prefix("ctx.").unwrap_or(loc);
        let root = loc.split('.').next().unwrap_or("");
        self.var(root).is_some() || self.ctx.get(root).is_some()
    }

    fn exec_script(&mut self, script: &str, _states: &dyn StateQuery) -> Result<(), EvalError> {
        let s = script.trim();
        if !lowering::is_iri(s) {
            return Err(EvalError::new(
                "the rust datamodel runs only action IRIs as scripts",
            ));
        }
        let emitted = self.run_action(s)?;
        if emitted.is_empty() {
            Ok(())
        } else {
            Err(EvalError::new(format!(
                "action `{s}` emitted events from a <script>; lower actions to <foreach> so \
                 their events are sent"
            )))
        }
    }

    fn foreach_items(
        &mut self,
        array: &str,
        states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        let a = array.trim();
        if lowering::is_iri(a) {
            return self.run_action(a);
        }
        match self.eval_value(a, states)? {
            Value::Array(items) => Ok(items),
            _ => Err(EvalError::new(format!("`{a}` is not a list"))),
        }
    }

    // @tg: implements:: [[openspec:rust-dsl#Rust Datamodel]]
    fn snapshot(&self) -> Value {
        Value::Object(vec![
            ("context".to_string(), self.ctx.snapshot()),
            ("vars".to_string(), Value::Object(self.vars.clone())),
        ])
    }

    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        let Value::Object(entries) = snapshot else {
            return Err(EvalError::new("a rust datamodel snapshot is an object"));
        };
        let get = |k: &str| entries.iter().find(|(n, _)| n == k).map(|(_, v)| v);
        let ctx = C::restore(get("context").unwrap_or(&Value::Undefined))?;
        let vars = match get("vars") {
            Some(Value::Object(v)) => v.clone(),
            None => Vec::new(),
            Some(_) => return Err(EvalError::new("snapshot `vars` is not an object")),
        };
        self.ctx = ctx;
        self.vars = vars;
        Ok(())
    }
}

/// A datamodel registry that creates `rust` datamodels over `C` with the functions of one
/// [`Registry`].
pub struct RustRegistry<C> {
    registry: Arc<Registry<C>>,
}

impl<C> RustRegistry<C> {
    /// A datamodel registry for `registry`.
    pub fn new(registry: Arc<Registry<C>>) -> Self {
        RustRegistry { registry }
    }
}

impl<C: Context> DatamodelRegistry for RustRegistry<C> {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        (name == RUST_DATAMODEL)
            .then(|| Box::new(RustDatamodel::new(self.registry.clone())) as Box<dyn Datamodel>)
    }
}

/// Types the expressions of the `rust` datamodel for the core chart checker. Guard and action
/// IRIs are typed by the Rust compiler through the generated bindings (and at registration),
/// so they are opaque here; `In()` and negations are `bool`; system and loop variables are
/// dynamic; literals and context paths are typed by [`BasicTyper`].
// @tg: implements:: [[openspec:rust-dsl#Rust Datamodel]]
#[derive(Clone, Debug)]
pub struct RustTyper {
    basic: BasicTyper,
}

impl Default for RustTyper {
    fn default() -> Self {
        RustTyper {
            basic: BasicTyper::for_datamodel(RUST_DATAMODEL),
        }
    }
}

impl ExprTyper for RustTyper {
    fn type_of(&self, expr: &str, scope: &Scope<'_>) -> Result<ExprType, Vec<ExprError>> {
        let e = expr.trim();
        let bare = e.trim_start_matches(['!', ' ']);
        if parse_in_predicate(bare).is_some() || (e.starts_with('!') && lowering::is_iri(bare)) {
            return Ok(ExprType::Known(Type::BOOL));
        }
        if lowering::is_iri(bare) || e.starts_with('_') {
            return Ok(ExprType::Unknown);
        }
        self.basic.type_of(e, scope)
    }
}

// ---- literals ----------------------------------------------------------------------------

/// Parse a JSON literal (single-quoted strings allowed). Numbers that an `f64` holds exactly
/// as written become numbers; longer decimals stay exact as decimal strings.
pub fn parse_literal(text: &str) -> Option<Value> {
    let mut p = Lit {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

struct Lit<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lit<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self, w: &str) -> bool {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            let end = self.i + w.len();
            if self
                .s
                .get(end)
                .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
            {
                self.i = end;
                return true;
            }
        }
        false
    }

    fn value(&mut self) -> Option<Value> {
        self.ws();
        match *self.s.get(self.i)? {
            b'"' | b'\'' => self.string().map(Value::String),
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(Value::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b']') {
                        return Some(Value::Array(items));
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            b'{' => {
                self.i += 1;
                let mut entries = Vec::new();
                if self.eat(b'}') {
                    return Some(Value::Object(entries));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    if !self.eat(b':') {
                        return None;
                    }
                    entries.push((k, self.value()?));
                    if self.eat(b'}') {
                        return Some(Value::Object(entries));
                    }
                    if !self.eat(b',') {
                        return None;
                    }
                }
            }
            b't' if self.word("true") => Some(Value::Bool(true)),
            b'f' if self.word("false") => Some(Value::Bool(false)),
            b'n' if self.word("null") => Some(Value::Null),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                self.i += 1;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.i += 1;
                }
                let text = std::str::from_utf8(&self.s[start..self.i]).ok()?;
                let d: Decimal = text.parse().ok()?;
                let f = d.to_f64();
                Some(if Decimal::from_f64(f) == Some(d) {
                    Value::Number(f)
                } else {
                    Value::String(d.to_string())
                })
            }
            _ => None,
        }
    }

    fn string(&mut self) -> Option<String> {
        let q = *self.s.get(self.i)?;
        if q != b'"' && q != b'\'' {
            return None;
        }
        self.i += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                c if c == q => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                            self.i += 4;
                            let mut buf = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        assert_eq!(parse_literal("0"), Some(Value::Number(0.0)));
        assert_eq!(parse_literal("-1.5"), Some(Value::Number(-1.5)));
        assert_eq!(
            parse_literal("12345678901234567890.25"),
            Some(Value::String("12345678901234567890.25".into()))
        );
        assert_eq!(parse_literal("'a\\'b'"), Some(Value::String("a'b".into())));
        assert_eq!(
            parse_literal(r#"{"a": [1, true, null]}"#),
            Some(Value::Object(vec![(
                "a".into(),
                Value::Array(vec![Value::Number(1.0), Value::Bool(true), Value::Null])
            )]))
        );
        assert_eq!(parse_literal("ctx.total"), None);
        assert_eq!(parse_literal("truex"), None);
    }
}
