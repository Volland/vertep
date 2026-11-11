//! The `null` datamodel (W3C SCXML B.1) and the registry helpers that make it always available.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::compile::CompiledChart;
use crate::datamodel::{
    Datamodel, DatamodelRegistry, Environment, EvalError, Event, SessionInfo, StateQuery, Value,
};
use crate::session::{Outcome, StartError, StartInput, start};

/// The name of the `null` datamodel.
pub const NULL_DATAMODEL: &str = "null";

/// The `null` datamodel: no data, and only `In('id')` conditions. Every other condition, value
/// expression, assignment, script and `foreach` is an evaluation error. Inline content (of
/// `<content>`) is not an expression and is passed through as a string.
// @tg: implements:: [[openspec:datamodel#Null Datamodel]]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NullDatamodel;

impl NullDatamodel {
    /// A new `null` datamodel.
    pub fn new() -> Self {
        NullDatamodel
    }
}

/// Parse `In('id')` or `In("id")` (surrounding whitespace allowed) and return `id`.
// @tg: implements:: [[openspec:datamodel#In Predicate]]
pub fn parse_in_predicate(expr: &str) -> Option<&str> {
    let e = expr.trim();
    let inner = e.strip_prefix("In")?.trim_start();
    let inner = inner.strip_prefix('(')?.strip_suffix(')')?.trim();
    let quote = inner.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let id = inner.strip_prefix(quote)?.strip_suffix(quote)?;
    if id.is_empty() || id.contains(quote) {
        return None;
    }
    Some(id)
}

fn unsupported(what: &str) -> EvalError {
    EvalError::new(alloc::format!("the null datamodel does not support {what}"))
}

impl Datamodel for NullDatamodel {
    fn name(&self) -> &str {
        NULL_DATAMODEL
    }

    fn init_session(&mut self, _info: &SessionInfo) -> Result<(), EvalError> {
        Ok(())
    }

    fn set_environment(&mut self, _env: &Environment) {}

    fn declare(&mut self, _id: &str) -> Result<(), EvalError> {
        Err(unsupported("data"))
    }

    fn set_event(&mut self, _event: &Event) {}

    fn eval_cond(&mut self, expr: &str, states: &dyn StateQuery) -> Result<bool, EvalError> {
        match parse_in_predicate(expr) {
            Some(id) => Ok(states.in_state(id)),
            None => Err(unsupported("conditions other than In()")),
        }
    }

    fn eval_value(&mut self, _expr: &str, _states: &dyn StateQuery) -> Result<Value, EvalError> {
        Err(unsupported("value expressions"))
    }

    fn eval_content(&mut self, content: &str) -> Result<Value, EvalError> {
        Ok(Value::String(String::from(content)))
    }

    fn assign_expr(
        &mut self,
        _location: &str,
        _expr: &str,
        _states: &dyn StateQuery,
    ) -> Result<(), EvalError> {
        Err(unsupported("assign"))
    }

    fn assign_value(&mut self, _location: &str, _value: Value) -> Result<(), EvalError> {
        Err(unsupported("assign"))
    }

    fn bind_var(&mut self, _name: &str, _value: Value) -> Result<(), EvalError> {
        Err(unsupported("variables"))
    }

    fn is_location(&mut self, _location: &str) -> bool {
        false
    }

    fn exec_script(&mut self, _script: &str, _states: &dyn StateQuery) -> Result<(), EvalError> {
        Err(unsupported("script"))
    }

    fn foreach_items(
        &mut self,
        _array: &str,
        _states: &dyn StateQuery,
    ) -> Result<Vec<Value>, EvalError> {
        Err(unsupported("foreach"))
    }

    /// The `null` datamodel has no state: its snapshot is the empty value (undefined).
    // @tg: implements:: [[openspec:datamodel#Datamodel Snapshot]] {confidence: partial, note: "null datamodel only"}
    fn snapshot(&self) -> Value {
        Value::Undefined
    }

    fn restore(&mut self, snapshot: &Value) -> Result<(), EvalError> {
        match snapshot {
            Value::Undefined => Ok(()),
            _ => Err(EvalError::new("a null datamodel snapshot is undefined")),
        }
    }
}

/// A registry with only the `null` datamodel.
#[derive(Clone, Copy, Debug, Default)]
pub struct NullRegistry;

impl DatamodelRegistry for NullRegistry {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        if name == NULL_DATAMODEL || name.is_empty() {
            Some(Box::new(NullDatamodel))
        } else {
            None
        }
    }
}

/// Wraps a host registry so that `null` is always available.
#[derive(Clone, Debug, Default)]
pub struct WithNull<R>(pub R);

impl<R: DatamodelRegistry> DatamodelRegistry for WithNull<R> {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        self.0.create(name).or_else(|| NullRegistry.create(name))
    }
}

/// Create the datamodel a chart names (`null` when it names none) from the host's registry, or
/// fail with [`StartError::UnsupportedDatamodel`]. `null` is available even when the registry
/// does not list it.
// @tg: implements:: [[openspec:datamodel#Datamodel Registry]]
pub fn create_datamodel(
    registry: &dyn DatamodelRegistry,
    name: &str,
) -> Result<Box<dyn Datamodel>, StartError> {
    let name = if name.is_empty() {
        NULL_DATAMODEL
    } else {
        name
    };
    registry
        .create(name)
        .or_else(|| NullRegistry.create(name))
        .ok_or_else(|| StartError::UnsupportedDatamodel {
            name: String::from(name),
        })
}

/// Select the chart's datamodel from the registry and start a session with it. Returns the
/// outcome and the datamodel instance the session must keep using.
pub fn start_with_registry(
    chart: &CompiledChart,
    registry: &dyn DatamodelRegistry,
    input: StartInput,
) -> Result<(Outcome, Box<dyn Datamodel>), StartError> {
    let mut dm = create_datamodel(registry, chart.datamodel())?;
    let outcome = start(chart, input, dm.as_mut())?;
    Ok((outcome, dm))
}
