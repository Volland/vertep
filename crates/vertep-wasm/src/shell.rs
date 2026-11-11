//! The browser shell: the runtime's spawn, clock and sleep contract on the JavaScript event
//! loop. It works unchanged in a browser's dedicated worker and in a Node.js worker thread,
//! which both provide `setTimeout` and `Date.now()`.
//!
//! - spawn: `wasm_bindgen_futures::spawn_local` (the worker has one thread).
//! - clock: `Date.now()`, read only by the shell when it stamps an input; the step never reads
//!   it, it sees the journaled timestamp.
//! - sleep: a future completed by a `setTimeout` callback. The future holds only shared state,
//!   so it is `Send` as the runtime requires, without any `unsafe`.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use vertep_runtime::shell::{BoxFuture, Clock, Shell, Sleep, Spawn};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// Spawns on the worker's event loop.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalSpawn;

impl Spawn for LocalSpawn {
    fn spawn(&self, fut: BoxFuture<()>) {
        wasm_bindgen_futures::spawn_local(fut);
    }
}

/// `Date.now()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DateClock;

impl Clock for DateClock {
    fn now_ms(&self) -> i64 {
        js_sys::Date::now() as i64
    }
}

#[derive(Default)]
struct TimerState {
    fired: bool,
    waker: Option<Waker>,
}

/// Sleeps with `setTimeout` until a time of `Date.now()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeoutSleep;

struct TimeoutFuture {
    state: Arc<Mutex<TimerState>>,
}

impl Future for TimeoutFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut g = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if g.fired {
            Poll::Ready(())
        } else {
            g.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}

/// Calls the global `setTimeout(callback, ms)`.
fn set_timeout(callback: &JsValue, ms: f64) -> Result<(), JsValue> {
    let global = js_sys::global();
    let f: js_sys::Function = js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout"))?
        .dyn_into()
        .map_err(|_| JsValue::from_str("setTimeout is not a function"))?;
    f.call2(&global, callback, &JsValue::from_f64(ms))?;
    Ok(())
}

impl Sleep for TimeoutSleep {
    fn sleep_until(&self, at_ms: i64) -> BoxFuture<()> {
        let state = Arc::new(Mutex::new(TimerState::default()));
        let wait = at_ms.saturating_sub(DateClock.now_ms()).max(0);
        if wait == 0 {
            state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fired = true;
        } else {
            let s = state.clone();
            let cb = Closure::once_into_js(move || {
                let mut g = s.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                g.fired = true;
                if let Some(w) = g.waker.take() {
                    w.wake();
                }
            });
            if set_timeout(&cb, wait as f64).is_err() {
                // no timer source: complete at once rather than never
                state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .fired = true;
            }
        }
        Box::pin(TimeoutFuture { state })
    }
}

/// The browser (and Node.js worker-thread) shell.
// @tg: implements:: [[openspec:wasm-bindings#Browser Runtime Shell]]
// @tg: implements:: [[openspec:wasm-bindings#Node.js Support]] {confidence: partial, note: "the same shell on Node.js timers"}
pub fn browser_shell() -> Shell {
    Shell {
        spawn: Arc::new(LocalSpawn),
        clock: Arc::new(DateClock),
        sleep: Arc::new(TimeoutSleep),
    }
}
