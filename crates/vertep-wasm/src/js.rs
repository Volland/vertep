//! The wasm-bindgen surface the worker entry (`@vertep/web/worker`) calls. Errors are thrown as
//! `Error` objects whose message is `vertep:{"code","message","violations"?}`; the client turns
//! them into typed `VertepError`s.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::Value as J;
use vertep_runtime::Address;
use vertep_runtime::shell::Sleep;
use wasm_bindgen::prelude::*;

use crate::error::{ApiError, Code};
use crate::functions::{ActionOutcome, Implementations};
use crate::shell::{TimeoutSleep, browser_shell};
use crate::store::{StorageKind, Store};
use crate::system::{System, SystemOptions, event_from_json};
use crate::trace::{TraceCase, trace_all};

fn thrown(e: &ApiError) -> JsError {
    JsError::new(&format!("vertep:{}", e.to_json()))
}

fn parse(text: &str, what: &str) -> Result<J, JsError> {
    serde_json::from_str(text)
        .map_err(|e| thrown(&ApiError::arg(format!("{what} is not JSON: {e}"))))
}

/// Guards and actions implemented by two JavaScript functions the worker builds from the
/// application's implementations module: `guard(iri, contextJson, eventJson) -> boolean` and
/// `action(iri, contextJson, eventJson) -> resultJson`.
struct JsImplementations {
    guard: js_sys::Function,
    action: js_sys::Function,
}

fn js_error(e: JsValue) -> String {
    e.dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| e.as_string())
        .unwrap_or_else(|| "a JavaScript exception".into())
}

impl Implementations for JsImplementations {
    fn guard(&self, iri: &str, context: &J, event: &J) -> Result<bool, String> {
        let r = self
            .guard
            .call3(
                &JsValue::NULL,
                &JsValue::from_str(iri),
                &JsValue::from_str(&context.to_string()),
                &JsValue::from_str(&event.to_string()),
            )
            .map_err(js_error)?;
        Ok(r.is_truthy())
    }

    fn action(&self, iri: &str, context: &J, event: &J) -> Result<ActionOutcome, String> {
        let r = self
            .action
            .call3(
                &JsValue::NULL,
                &JsValue::from_str(iri),
                &JsValue::from_str(&context.to_string()),
                &JsValue::from_str(&event.to_string()),
            )
            .map_err(js_error)?;
        match r.as_string() {
            Some(text) => {
                let j: J = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                ActionOutcome::from_json(&j)
            }
            None if r.is_null() || r.is_undefined() => Ok(ActionOutcome::default()),
            None => Err("an action wrapper returns JSON text".into()),
        }
    }
}

use wasm_bindgen::JsCast;

fn implementations(v: &JsValue) -> Result<Option<Rc<dyn Implementations>>, JsError> {
    if v.is_null() || v.is_undefined() {
        return Ok(None);
    }
    let get = |k: &str| -> Result<js_sys::Function, JsError> {
        js_sys::Reflect::get(v, &JsValue::from_str(k))
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .ok_or_else(|| thrown(&ApiError::arg(format!("implementations.{k} is a function"))))
    };
    Ok(Some(Rc::new(JsImplementations {
        guard: get("guard")?,
        action: get("action")?,
    })))
}

/// An actor system inside the worker.
#[wasm_bindgen]
pub struct ActorSystem {
    inner: Rc<RefCell<System>>,
    notify: Rc<RefCell<Option<js_sys::Function>>>,
    closed: Rc<Cell<bool>>,
    kick: futures::channel::mpsc::UnboundedSender<()>,
}

fn flush(system: &mut System, notify: &RefCell<Option<js_sys::Function>>) {
    let items = system.take_notifications();
    if items.is_empty() {
        return;
    }
    let f = notify.borrow().clone();
    if let Some(f) = f {
        for n in items {
            let _ = f.call1(&JsValue::NULL, &JsValue::from_str(&n.to_string()));
        }
    }
}

/// Drives the node on the event loop: run until idle, deliver notifications, then wait for new
/// work, a kick from a command, or the next deadline (a timer, an idle deactivation).
///
/// The first scheduling round, which recovers what an earlier run left (overdue durable timers
/// among it), waits for the first command: only then are the application's chart types
/// registered, so an overdue timer fires once, after the journal is open and its chart known.
fn drive(
    inner: Rc<RefCell<System>>,
    notify: Rc<RefCell<Option<js_sys::Function>>>,
    closed: Rc<Cell<bool>>,
    mut kicks: futures::channel::mpsc::UnboundedReceiver<()>,
) {
    use futures::StreamExt;
    wasm_bindgen_futures::spawn_local(async move {
        let mut first = true;
        loop {
            if closed.get() {
                break;
            }
            let (deadline, wake) = match inner.try_borrow_mut() {
                Ok(mut s) => {
                    if !first {
                        s.pump();
                        flush(&mut s, &notify);
                    }
                    (s.next_deadline(), s.handle().wakeup())
                }
                Err(_) => (Some(js_sys::Date::now() as i64 + 1), {
                    let s = inner.borrow();
                    s.handle().wakeup()
                }),
            };
            let kick = kicks.next();
            let woken = futures::future::select(wake, kick);
            match deadline.filter(|_| !first) {
                Some(t) => {
                    futures::future::select(woken, TimeoutSleep.sleep_until(t)).await;
                }
                None => {
                    woken.await;
                }
            }
            first = false;
        }
    });
}

#[wasm_bindgen]
impl ActorSystem {
    /// Creates an actor system: `options` is `{storage: "memory" | "opfs", path, journal,
    /// dev, idleTimeoutMs, guards, actions}`; `implementations` is null or the wrapper object
    /// with `guard` and `action`; `import` is a journal file to start from. OPFS is installed
    /// first (dedicated worker only). Overdue durable timers fire once, after the journal is
    /// open, on the first scheduling round.
    pub async fn create(
        options: String,
        implementations_obj: JsValue,
        import: Option<Vec<u8>>,
    ) -> Result<ActorSystem, JsError> {
        install_panic_hook();
        let opts =
            SystemOptions::from_json(&parse(&options, "the options")?).map_err(|e| thrown(&e))?;
        if opts.store.kind == StorageKind::Opfs {
            install_opfs(&opts).await?;
        }
        let store = Store::open(&opts.store, import.as_deref()).map_err(|e| thrown(&e))?;
        let imp = implementations(&implementations_obj)?;
        let system = System::new(&opts, store, browser_shell(), imp).map_err(|e| thrown(&e))?;
        let (kick, kicks) = futures::channel::mpsc::unbounded();
        let s = ActorSystem {
            inner: Rc::new(RefCell::new(system)),
            notify: Rc::new(RefCell::new(None)),
            closed: Rc::new(Cell::new(false)),
            kick,
        };
        drive(s.inner.clone(), s.notify.clone(), s.closed.clone(), kicks);
        Ok(s)
    }

    /// Sets the notification callback, called with each notification as JSON text.
    #[wasm_bindgen(js_name = onNotify)]
    pub fn on_notify(&self, f: js_sys::Function) {
        *self.notify.borrow_mut() = Some(f);
    }

    /// Runs a command (see `System::command`) and returns its result as JSON text.
    pub fn command(&self, op: String, args: String) -> Result<String, JsError> {
        let args = parse(&args, "the arguments")?;
        let mut s = self
            .inner
            .try_borrow_mut()
            .map_err(|_| thrown(&ApiError::arg("the actor system is busy (re-entrant call)")))?;
        let r = s.command(&op, &args);
        flush(&mut s, &self.notify);
        // the command may have armed timers: let the driver recompute its deadline
        let _ = self.kick.unbounded_send(());
        r.map(|j| j.to_string()).map_err(|e| thrown(&e))
    }

    /// Sends an event and resolves with the reply event as JSON text, or rejects with
    /// `RequestTimeout` after `timeout_ms`.
    pub fn request(&self, to: String, event: String, timeout_ms: f64) -> js_sys::Promise {
        let handle = self.inner.borrow().handle();
        wasm_bindgen_futures::future_to_promise(async move {
            let to =
                Address::parse(&to).map_err(|e| JsValue::from(thrown(&ApiError::arg(e.reason))))?;
            let ev = serde_json::from_str::<J>(&event)
                .map_err(|e| ApiError::arg(e.to_string()))
                .and_then(|j| event_from_json(&j))
                .map_err(|e| JsValue::from(thrown(&e)))?;
            let timeout = if timeout_ms.is_finite() && timeout_ms > 0.0 {
                timeout_ms as u64
            } else {
                30_000
            };
            crate::system::request(handle, to, ev, timeout)
                .await
                .map(|j| JsValue::from_str(&j.to_string()))
                .map_err(|e| JsValue::from(thrown(&e)))
        })
    }

    /// The journal file's bytes (journaled stores only).
    #[wasm_bindgen(js_name = exportJournal)]
    pub fn export_journal(&self) -> Result<Vec<u8>, JsError> {
        self.inner.borrow().store().export().map_err(|e| thrown(&e))
    }

    /// Stops the driver; the store is released when the system is dropped.
    pub fn close(&self) {
        self.closed.set(true);
        let _ = self.kick.unbounded_send(());
        self.inner.borrow().handle().shutdown();
    }
}

/// Reports Rust panics on the console instead of a bare `unreachable` trap.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            web_sys::console::error_1(&JsValue::from_str(&format!("vertep panicked: {info}")));
        }));
    });
}

#[cfg(feature = "journal")]
async fn install_opfs(opts: &SystemOptions) -> Result<(), JsError> {
    let o = tm_wasm::OpfsOptions {
        directory: opts
            .store
            .path
            .rsplit_once('/')
            .map(|(d, _)| d)
            .filter(|d| !d.is_empty())
            .unwrap_or("/vertep")
            .to_owned(),
        ..tm_wasm::OpfsOptions::default()
    };
    tm_wasm::install_opfs(&o).await.map_err(|e| {
        thrown(&ApiError::new(
            Code::StorageUnavailable,
            format!("OPFS sync access handles are unavailable here: {e}"),
        ))
    })
}

#[cfg(not(feature = "journal"))]
async fn install_opfs(_opts: &SystemOptions) -> Result<(), JsError> {
    Err(thrown(&ApiError::new(
        Code::StorageUnavailable,
        "this build has no journal storage; OPFS needs the build with the tiramemsu journal",
    )))
}

/// Steps chart cases with the pure core (`null` and `cel` datamodels) and returns the traces as
/// JSON text, for comparison with the native build.
#[wasm_bindgen(js_name = traceCases)]
pub fn trace_cases(cases: String) -> Result<String, JsError> {
    install_panic_hook();
    let cases: Vec<TraceCase> =
        serde_json::from_str(&cases).map_err(|e| thrown(&ApiError::arg(format!("cases: {e}"))))?;
    let registry = crate::functions::Datamodels::new(std::sync::Arc::new(Default::default()));
    Ok(trace_all(&cases, &registry))
}

/// Runs generated differential cases (`[{chart, inputs}]`, the chart as IR JSON) exactly as
/// the native Rust-versus-Lean differential test does, and returns their oracle records as
/// JSON text.
#[wasm_bindgen(js_name = differentialRecords)]
pub fn differential_records(cases: String) -> Result<String, JsError> {
    install_panic_hook();
    let cases: Vec<crate::trace::DiffCase> =
        serde_json::from_str(&cases).map_err(|e| thrown(&ApiError::arg(format!("cases: {e}"))))?;
    let registry = crate::functions::Datamodels::new(std::sync::Arc::new(Default::default()));
    Ok(crate::trace::diff_all(&cases, &registry))
}

/// The crate version the package was built from.
#[wasm_bindgen]
pub fn version() -> String {
    crate::VERSION.to_owned()
}

/// Whether this build contains the tiramemsu journal.
#[wasm_bindgen(js_name = hasJournal)]
pub fn has_journal() -> bool {
    cfg!(feature = "journal")
}
