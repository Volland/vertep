//! Named guards and actions supplied by the host (JavaScript functions from an implementations
//! module in the worker, or Rust closures in native tests), bound into the `rust`-style
//! named-function registry of `vertep-dm-rust`, plus the datamodel registry of a system.
//!
//! The registry stores `Send + Sync` closures; JavaScript functions are neither, so each closure
//! captures only the function IRI and the system's slot number and calls through a
//! thread-local table. Every call is synchronous, inside the step, on the worker's thread.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Map, Value as J};
use vertep_core::datamodel::{Datamodel, DatamodelRegistry, Event, Value};
use vertep_dm_rust::datamodel::{Context, DynContext, RustRegistry};
use vertep_dm_rust::emit::{Emit, SendOptions};
use vertep_dm_rust::registry::Registry;

/// One event an action emits: raised on the internal queue, or sent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Emitted {
    /// The event name.
    pub name: String,
    /// The payload.
    pub data: J,
    /// Raise (internal queue, same macrostep) rather than send.
    pub raise: bool,
    /// The send target; the session itself when absent.
    pub target: Option<String>,
    /// The send delay in milliseconds.
    pub delay_ms: u64,
}

/// What an action returns: the context fields it changed and the events it emitted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionOutcome {
    /// The new values of changed context fields.
    pub context: Map<String, J>,
    /// Emitted events, in order.
    pub emitted: Vec<Emitted>,
}

impl ActionOutcome {
    /// Reads `{context?: {...}, raise?: [{name, data}], send?: [{name, data, target, delay}]}`.
    ///
    /// # Errors
    ///
    /// A message when the shape is wrong.
    pub fn from_json(j: &J) -> Result<Self, String> {
        let mut out = ActionOutcome::default();
        if j.is_null() {
            return Ok(out);
        }
        let o = j.as_object().ok_or("an action result is an object")?;
        if let Some(c) = o.get("context").filter(|c| !c.is_null()) {
            out.context = c
                .as_object()
                .ok_or("an action result's context is an object")?
                .clone();
        }
        for (key, raise) in [("raise", true), ("send", false)] {
            let Some(list) = o.get(key).filter(|l| !l.is_null()) else {
                continue;
            };
            for e in list.as_array().ok_or("raise and send are arrays")? {
                let name = e
                    .get("name")
                    .or_else(|| e.get("type"))
                    .and_then(J::as_str)
                    .ok_or("an emitted event has a name")?
                    .to_owned();
                out.emitted.push(Emitted {
                    name,
                    data: e.get("data").cloned().unwrap_or(J::Null),
                    raise,
                    target: e.get("target").and_then(J::as_str).map(str::to_owned),
                    delay_ms: e.get("delay").and_then(J::as_u64).unwrap_or(0),
                });
            }
        }
        Ok(out)
    }
}

/// Named guards and actions of one actor system.
pub trait Implementations {
    /// Evaluates guard `iri` on the context (an object of fields) and the event.
    ///
    /// # Errors
    ///
    /// A message when the guard is missing or throws.
    fn guard(&self, iri: &str, context: &J, event: &J) -> Result<bool, String>;

    /// Runs action `iri` on the context and the event.
    ///
    /// # Errors
    ///
    /// A message when the action is missing, throws or returns a malformed result.
    fn action(&self, iri: &str, context: &J, event: &J) -> Result<ActionOutcome, String>;
}

thread_local! {
    static SLOTS: RefCell<BTreeMap<u64, Rc<dyn Implementations>>> = RefCell::new(BTreeMap::new());
    static NEXT: RefCell<u64> = const { RefCell::new(1) };
}

/// The thread-local slot of one system's implementations, freed on drop.
pub struct Slot(u64);

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Slot").field(&self.0).finish()
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        SLOTS.with(|s| s.borrow_mut().remove(&self.0));
    }
}

fn with_slot<T>(slot: u64, f: impl FnOnce(&dyn Implementations) -> T) -> Option<T> {
    let imp = SLOTS.with(|s| s.borrow().get(&slot).cloned())?;
    Some(f(imp.as_ref()))
}

/// A Vertep value as plain JSON: `undefined` and non-finite numbers become `null`.
pub fn value_to_json(v: &Value) -> J {
    match v {
        Value::Undefined | Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Number(n) => serde_json::Number::from_f64(*n).map_or(J::Null, J::Number),
        Value::String(s) => J::String(s.clone()),
        Value::Array(a) => J::Array(a.iter().map(value_to_json).collect()),
        Value::Object(o) => J::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect(),
        ),
    }
}

/// Plain JSON as a Vertep value.
pub fn json_to_value(j: &J) -> Value {
    match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        J::String(s) => Value::String(s.clone()),
        J::Array(a) => Value::Array(a.iter().map(json_to_value).collect()),
        J::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect(),
        ),
    }
}

/// An event as the host sees it: `{name, data, origin, origintype, sendid, invokeid}`, plus
/// `originverified: true` when the shell verified the sender DID.
pub fn event_to_json(e: &Event) -> J {
    let mut o = Map::new();
    o.insert("name".into(), J::String(e.name.clone()));
    o.insert("data".into(), value_to_json(&e.data));
    for (k, v) in [
        ("origin", &e.origin),
        ("origintype", &e.origintype),
        ("sendid", &e.sendid),
        ("invokeid", &e.invokeid),
    ] {
        if let Some(v) = v {
            o.insert(k.into(), J::String(v.clone()));
        }
    }
    if e.originverified {
        o.insert("originverified".into(), J::Bool(true));
    }
    J::Object(o)
}

/// Binds `guards` and `actions` (IRIs as the charts write them) to `imp` and returns the
/// registry together with the slot that keeps `imp` reachable.
///
/// # Errors
///
/// A message when an IRI is listed twice.
// @tg: implements:: [[openspec:wasm-bindings#Worker-Hosted Actor System]] {confidence: partial, note: "the Rust half of implementations loaded in the worker"}
pub fn bind(
    imp: Rc<dyn Implementations>,
    guards: &[String],
    actions: &[String],
) -> Result<(Arc<Registry<DynContext>>, Slot), String> {
    let id = NEXT.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n += 1;
        id
    });
    SLOTS.with(|s| s.borrow_mut().insert(id, imp));
    let slot = Slot(id);
    let mut reg: Registry<DynContext> = Registry::new();
    for iri in guards {
        let name = iri.clone();
        reg.guard::<Event, _>(iri, move |ctx: &DynContext, ev: &Event| {
            let c = value_to_json(&ctx.snapshot());
            let e = event_to_json(ev);
            with_slot(id, |i| i.guard(&name, &c, &e))
                .unwrap_or(Err("the actor system is closed".into()))
                .unwrap_or(false)
        })
        .map_err(|e| e.to_string())?;
    }
    for iri in actions {
        let name = iri.clone();
        reg.action::<Event, (), _, _>(
            iri,
            move |ctx: &mut DynContext, ev: &Event, emit: &mut Emit<()>| {
                let c = value_to_json(&ctx.snapshot());
                let e = event_to_json(ev);
                let out = match with_slot(id, |i| i.action(&name, &c, &e)) {
                    Some(Ok(o)) => o,
                    Some(Err(msg)) => {
                        emit.raise_untyped(
                            "error.execution",
                            Value::String(format!("action {name}: {msg}")),
                        );
                        return;
                    }
                    None => return,
                };
                for (k, v) in &out.context {
                    let _ = ctx.declare(k);
                    let _ = ctx.set(k, &json_to_value(v));
                }
                for em in out.emitted {
                    let data = json_to_value(&em.data);
                    if em.raise {
                        emit.raise_untyped(&em.name, data);
                    } else {
                        let options = SendOptions {
                            target: em.target,
                            delay_ms: em.delay_ms,
                            ..SendOptions::default()
                        };
                        emit.send_untyped(&em.name, data, options);
                    }
                }
            },
        )
        .map_err(|e| e.to_string())?;
    }
    Ok((Arc::new(reg), slot))
}

/// The datamodels a system offers: `null` (always), `cel`, `rust` (named functions from the
/// implementations), and `ecmascript` when built with the `ecmascript` feature.
pub struct Datamodels {
    rust: RustRegistry<DynContext>,
}

impl std::fmt::Debug for Datamodels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Datamodels").finish_non_exhaustive()
    }
}

impl Datamodels {
    /// The registry with the `rust` datamodel calling `functions`.
    pub fn new(functions: Arc<Registry<DynContext>>) -> Self {
        Datamodels {
            rust: RustRegistry::new(functions),
        }
    }
}

impl DatamodelRegistry for Datamodels {
    fn create(&self, name: &str) -> Option<Box<dyn Datamodel>> {
        match name {
            vertep_dm_cel::CEL => vertep_dm_cel::create(name),
            vertep_dm_rust::datamodel::RUST_DATAMODEL => self.rust.create(name),
            #[cfg(feature = "ecmascript")]
            "ecmascript" => vertep_dm_ecma::EcmaRegistry.create(name),
            _ => None,
        }
    }
}
