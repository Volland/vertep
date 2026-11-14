//! Vertep for Python: the native half of the `vertep` package.
//!
//! The extension module `vertep._vertep` exposes one class, [`Core`], over
//! [`vertep_wasm::System`], the actor system behind the JavaScript bindings. Its commands are
//! `(op, JSON arguments) -> JSON result`, so Python, JavaScript and the native tests drive the
//! same operations with the same semantics; the ergonomic Python API (`ActorSystem`, `Actor`,
//! `Snapshot`, typed errors) is plain Python in `bindings/python/python/vertep`.
//!
//! Time is explicit. A `Core` is on the *system* clock (timers fire while Python waits in
//! `wait`) or on a *virtual* clock (timers fire only when Python advances it), the second being
//! deterministic and what tests want. Either way the step never reads a clock: it sees the
//! timestamp the journal recorded.
//!
//! A `Core` is single-threaded, because actor datamodels need not be `Send`: it is `unsendable`
//! and raises `RuntimeError` when another thread touches it.
//!
//! @tg: implements:: [[decisions#Python bindings wrap the command bridge]]
//! @tg: implements:: [[decisions#No-std core and tokio shell]] {confidence: partial, note: "native shell with a driven clock"}
//! @tg: implements:: [[decisions#Event-sourced actors]] {confidence: partial, note: "journals written from Python replay in Rust and JavaScript"}

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};
use serde_json::{Value as J, json};
use vertep_runtime::{Clock, Shell, VirtualClock};
use vertep_wasm::{
    ActionOutcome, ApiError, Implementations, StorageKind, Store, System, SystemOptions,
};

/// The crate version, which the Python package must carry.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The longest the interpreter stays out of reach of Ctrl-C while a `wait` sleeps.
const SLICE: Duration = Duration::from_millis(50);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Guards and actions that are Python callables, bound by IRI.
// @tg: implements:: [[openspec:python-bindings#Python Guards And Actions]]
struct PyImplementations {
    guards: HashMap<String, Py<PyAny>>,
    actions: HashMap<String, Py<PyAny>>,
    json: Py<PyModule>,
}

/// The event as a guard or action sees it, as in `@vertep/web`: `{type, ...fields, _meta}`.
fn user_event(e: &J) -> J {
    let mut out = serde_json::Map::new();
    out.insert("type".into(), e.get("name").cloned().unwrap_or(J::Null));
    match e.get("data") {
        Some(J::Object(fields)) => out.extend(fields.clone()),
        Some(J::Null) | None => {}
        Some(other) => {
            out.insert("data".into(), other.clone());
        }
    }
    let meta: serde_json::Map<String, J> = e
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| *k != "name" && *k != "data")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.insert("_meta".into(), J::Object(meta));
    J::Object(out)
}

/// An emitted event as the bridge reads it: `{name, data, target?, delay?}`. Python code may
/// write `{"type": "receipt", "amount": 5}` (the fields are the payload, as in `@vertep/web`) or
/// the bridge's own form.
fn emitted(e: &J) -> J {
    let Some(o) = e.as_object() else {
        return e.clone();
    };
    if o.contains_key("name") && o.contains_key("data") {
        return e.clone();
    }
    let name = o
        .get("name")
        .or_else(|| o.get("type"))
        .cloned()
        .unwrap_or(J::Null);
    let data: serde_json::Map<String, J> = o
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "name" | "type" | "target" | "delay"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut out = json!({ "name": name, "data": J::Object(data) });
    for k in ["target", "delay"] {
        if let Some(v) = o.get(k) {
            out[k] = v.clone();
        }
    }
    out
}

impl PyImplementations {
    /// Calls `f(context, event)` with plain Python values and returns its result as JSON.
    fn call(&self, f: &Py<PyAny>, context: &J, event: &J) -> Result<J, String> {
        Python::attach(|py| {
            let json = self.json.bind(py);
            let loads = json.getattr("loads").map_err(|e| e.to_string())?;
            let ctx = loads
                .call1((context.to_string(),))
                .map_err(|e| e.to_string())?;
            let ev = loads
                .call1((user_event(event).to_string(),))
                .map_err(|e| e.to_string())?;
            let out = f.call1(py, (ctx, ev)).map_err(|e| e.to_string())?;
            let text: String = json
                .getattr("dumps")
                .and_then(|d| d.call1((out,)))
                .and_then(|t| t.extract())
                .map_err(|e| format!("the result is not JSON: {e}"))?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        })
    }
}

impl Implementations for PyImplementations {
    fn guard(&self, iri: &str, context: &J, event: &J) -> Result<bool, String> {
        let f = self
            .guards
            .get(iri)
            .ok_or_else(|| format!("no guard bound to {iri}"))?;
        match self.call(f, context, event)? {
            J::Bool(b) => Ok(b),
            J::Null => Ok(false),
            J::Number(n) => Ok(n.as_f64().is_some_and(|f| f != 0.0)),
            J::String(s) => Ok(!s.is_empty()),
            J::Array(a) => Ok(!a.is_empty()),
            J::Object(o) => Ok(!o.is_empty()),
        }
    }

    fn action(&self, iri: &str, context: &J, event: &J) -> Result<ActionOutcome, String> {
        let f = self
            .actions
            .get(iri)
            .ok_or_else(|| format!("no action bound to {iri}"))?;
        let mut result = self.call(f, context, event)?;
        for key in ["raise", "send"] {
            if let Some(J::Array(list)) = result.get_mut(key) {
                *list = list.iter().map(emitted).collect();
            }
        }
        ActionOutcome::from_json(&result)
    }
}

/// The callables of a `{iri: callable}` dictionary.
fn callables(d: Option<&Bound<'_, PyDict>>) -> PyResult<HashMap<String, Py<PyAny>>> {
    let mut out = HashMap::new();
    if let Some(d) = d {
        for (k, v) in d.iter() {
            if !v.is_callable() {
                return Err(PyValueError::new_err(format!("{k} is not callable")));
            }
            out.insert(k.extract::<String>()?, v.unbind());
        }
    }
    Ok(out)
}

/// An actor system on one store, driven from Python.
// @tg: implements:: [[openspec:python-bindings#Native Extension Module]]
// @tg: implements:: [[openspec:python-bindings#Explicit Time]]
// @tg: implements:: [[openspec:python-bindings#Durable File Storage]]
#[pyclass(unsendable, module = "vertep._vertep")]
pub struct Core {
    system: Option<System>,
    clock: VirtualClock,
    wall: bool,
}

impl Core {
    fn system(&mut self) -> PyResult<&mut System> {
        self.system
            .as_mut()
            .ok_or_else(|| PyRuntimeError::new_err("the actor system is closed"))
    }

    /// Brings the clock to the wall clock; a virtual-time system keeps the time it was given.
    fn sync(&self) {
        if self.wall {
            self.clock.set(now_ms());
        }
    }
}

#[pymethods]
impl Core {
    /// Opens a system.
    ///
    /// `options` is the bridge's options object as JSON (`storage`, `path`, `dev`,
    /// `idleTimeoutMs`); `guards` and `actions` map IRIs to callables; `virtual_start` selects
    /// the virtual clock, starting at that many milliseconds.
    #[new]
    #[pyo3(signature = (options, guards=None, actions=None, virtual_start=None))]
    fn new(
        py: Python<'_>,
        options: &str,
        guards: Option<&Bound<'_, PyDict>>,
        actions: Option<&Bound<'_, PyDict>>,
        virtual_start: Option<i64>,
    ) -> PyResult<Core> {
        let mut options: J =
            serde_json::from_str(options).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let guards = callables(guards)?;
        let actions = callables(actions)?;
        options["guards"] = json!(guards.keys().collect::<Vec<_>>());
        options["actions"] = json!(actions.keys().collect::<Vec<_>>());
        let opts = SystemOptions::from_json(&options).map_err(bridge_error)?;
        // `file` is tiramemsu's native host: the file the Rust runtime opens, with the WAL.
        let store = if opts.store.kind == StorageKind::File {
            Store::Journal(std::sync::Arc::new(
                vertep_wasm::store::journal::JournalStore::native(&opts.store.path)
                    .map_err(bridge_error)?,
            ))
        } else {
            Store::open(&opts.store, None).map_err(bridge_error)?
        };
        let wall = virtual_start.is_none();
        let (shell, clock) = Shell::virtual_time(virtual_start.unwrap_or_else(now_ms));
        let imp: Option<Rc<dyn Implementations>> = if guards.is_empty() && actions.is_empty() {
            None
        } else {
            Some(Rc::new(PyImplementations {
                guards,
                actions,
                json: py.import("json")?.unbind(),
            }))
        };
        let system = System::new(&opts, store, shell, imp).map_err(bridge_error)?;
        Ok(Core {
            system: Some(system),
            clock,
            wall,
        })
    }

    // @tg: implements:: [[openspec:python-bindings#Actor Creation And Event Sending From Python]] {note: "the bridge command; the Python API encodes calls as commands"}
    // @tg: implements:: [[openspec:python-bindings#Journal Inspection And Time Travel]]
    /// Runs one command and returns `{"ok": result}` or `{"err": {code, message, violations?}}`
    /// as JSON; the Python layer turns the second into a typed exception.
    fn command(&mut self, op: &str, args: &str) -> PyResult<String> {
        self.sync();
        let args: J =
            serde_json::from_str(args).map_err(|e| PyValueError::new_err(e.to_string()))?;
        let reply = match self.system()?.command(op, &args) {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "err": e.to_json() }),
        };
        Ok(reply.to_string())
    }

    /// Runs everything that is due now and returns the notifications since the last call, as a
    /// JSON array (snapshots, transitions, deliveries, impurity reports).
    fn pump(&mut self) -> PyResult<String> {
        self.sync();
        let system = self.system()?;
        system.pump();
        Ok(J::Array(system.take_notifications()).to_string())
    }

    /// Moves the system forward by `ms` milliseconds and returns the notifications.
    ///
    /// On the virtual clock this advances time in one jump, firing every timer that falls due,
    /// in order. On the system clock it sleeps, with the interpreter free for other threads and
    /// interruptible by Ctrl-C, firing timers as they fall due.
    fn wait(&mut self, py: Python<'_>, ms: u64) -> PyResult<String> {
        let mut notes = Vec::new();
        if !self.wall {
            self.clock.advance(Duration::from_millis(ms));
            let system = self.system()?;
            system.pump();
            notes.extend(system.take_notifications());
            return Ok(J::Array(notes).to_string());
        }
        let end = now_ms().saturating_add(i64::try_from(ms).unwrap_or(i64::MAX));
        loop {
            self.sync();
            let system = self.system()?;
            system.pump();
            notes.extend(system.take_notifications());
            let now = now_ms();
            if now >= end {
                break;
            }
            let next = system.next_deadline().map_or(end, |d| d.min(end));
            let pause = Duration::from_millis(u64::try_from((next - now).max(1)).unwrap_or(1));
            py.detach(|| std::thread::sleep(pause.min(SLICE)));
            py.check_signals()?;
        }
        Ok(J::Array(notes).to_string())
    }

    /// The system's time in milliseconds since the epoch (virtual time on the virtual clock).
    fn now(&mut self) -> PyResult<i64> {
        self.sync();
        Ok(self.clock.now_ms())
    }

    /// When the next timer or deadline falls due, in milliseconds, if any.
    fn next_deadline(&mut self) -> PyResult<Option<i64>> {
        Ok(self.system()?.next_deadline())
    }

    /// The journal file's bytes (a journaled store only).
    fn export<'py>(&mut self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        let bytes = self.system()?.store().export().map_err(bridge_error)?;
        Ok(PyBytes::new(py, &bytes))
    }

    /// Whether the system runs on the wall clock.
    #[getter]
    fn wall_clock(&self) -> bool {
        self.wall
    }

    /// Closes the system and releases its store, so another process may open the file.
    fn close(&mut self) {
        self.system = None;
    }
}

/// A bridge error as a Python `ValueError` carrying its code (only used before the system
/// exists; afterwards errors travel as JSON).
fn bridge_error(e: ApiError) -> PyErr {
    PyValueError::new_err(e.to_json().to_string())
}

/// The extension module `vertep._vertep`.
#[pymodule]
fn _vertep(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Core>()?;
    m.add("__version__", VERSION)?;
    Ok(())
}
