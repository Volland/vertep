//! The browser end of the WebSocket Event I/O Processor: connects the system's [`Link`] to the
//! server with the platform's `WebSocket` (a browser's, or the global one of Node.js 22 and
//! later), presents the application's credential in `hello`, and reconnects with a doubling
//! delay. Every connection that the server welcomes sends the link's unacknowledged messages
//! again; received messages are delivered into local mailboxes and acknowledged.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use vertep_runtime::NodeHandle;
use vertep_runtime::shell::Sleep;
use vertep_runtime::websocket::{Frame, Inbound, Link};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use crate::shell::TimeoutSleep;
use crate::system::WebSocketOptions;

/// The longest reconnect delay, in milliseconds.
const MAX_BACKOFF_MS: i64 = 5_000;

enum Ev {
    Open,
    Text(String),
    Closed,
    Out(String),
}

/// What the actor system keeps of a running connection loop.
pub struct Connection {
    socket: Rc<RefCell<Option<WebSocket>>>,
    closed: Rc<Cell<bool>>,
}

impl Connection {
    /// Stops reconnecting and closes the open socket.
    pub fn close(&self) {
        self.closed.set(true);
        if let Some(ws) = self.socket.borrow_mut().take() {
            let _ = ws.close();
        }
    }
}

/// Asks the application's credential provider (a function returning a credential or a
/// Promise of one); `None` without a provider or when it fails.
async fn credential(provider: Option<&js_sys::Function>) -> Option<String> {
    let f = provider?;
    let v = f.call0(&JsValue::NULL).ok()?;
    let v = wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(&v))
        .await
        .ok()?;
    v.as_string()
}

/// Starts the connection loop of `link` to the server in `options`. `kick` wakes the actor
/// system's driver after a delivery or an acknowledgement, so notifications go out at once.
// @tg: implements:: [[openspec:wasm-bindings#Server Interop Over WebSocket]] {confidence: partial, note: "the browser and Node.js end on the platform WebSocket"}
pub fn connect(
    link: Arc<Link>,
    handle: NodeHandle,
    options: WebSocketOptions,
    provider: Option<js_sys::Function>,
    kick: UnboundedSender<()>,
) -> Connection {
    let socket: Rc<RefCell<Option<WebSocket>>> = Rc::new(RefCell::new(None));
    let closed = Rc::new(Cell::new(false));
    let c = Connection {
        socket: socket.clone(),
        closed: closed.clone(),
    };
    wasm_bindgen_futures::spawn_local(async move {
        let mut generation = 0_u64;
        let mut backoff = i64::from(options.reconnect_ms);
        while !closed.get() {
            let cred = credential(provider.as_ref()).await;
            if closed.get() {
                break;
            }
            let Ok(ws) = WebSocket::new(&options.url) else {
                TimeoutSleep
                    .sleep_until(js_sys::Date::now() as i64 + backoff)
                    .await;
                backoff = (backoff * 2).min(MAX_BACKOFF_MS);
                continue;
            };
            let (etx, erx) = unbounded::<Ev>();
            let on_open = Closure::<dyn FnMut()>::new({
                let etx = etx.clone();
                move || {
                    let _ = etx.unbounded_send(Ev::Open);
                }
            });
            let on_message = Closure::<dyn FnMut(MessageEvent)>::new({
                let etx = etx.clone();
                move |e: MessageEvent| {
                    if let Some(t) = e.data().as_string() {
                        let _ = etx.unbounded_send(Ev::Text(t));
                    }
                }
            });
            let on_close = Closure::<dyn FnMut(CloseEvent)>::new({
                let etx = etx.clone();
                move |_e: CloseEvent| {
                    let _ = etx.unbounded_send(Ev::Closed);
                }
            });
            let on_error = Closure::<dyn FnMut(web_sys::Event)>::new({
                let etx = etx.clone();
                move |_e: web_sys::Event| {
                    let _ = etx.unbounded_send(Ev::Closed);
                }
            });
            ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));
            ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
            ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));
            ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));
            *socket.borrow_mut() = Some(ws.clone());

            let (otx, orx) = unbounded::<String>();
            let mut events = futures::stream::select(erx, orx.map(Ev::Out));
            let mut welcomed = false;
            while let Some(ev) = events.next().await {
                match ev {
                    Ev::Open => {
                        let hello = Frame::Hello {
                            node: options.node.clone().unwrap_or_default(),
                            credential: cred.clone(),
                            hosts: options.hosts.clone(),
                        };
                        let _ = ws.send_with_str(&hello.to_text());
                    }
                    Ev::Out(t) => {
                        if ws.send_with_str(&t).is_err() {
                            break;
                        }
                    }
                    Ev::Text(t) => match link.receive(&t) {
                        Ok(Inbound::Handshake(Frame::Welcome { .. })) if !welcomed => {
                            welcomed = true;
                            generation += 1;
                            backoff = i64::from(options.reconnect_ms);
                            link.connected(generation, otx.clone());
                        }
                        Ok(Inbound::Deliver(env)) => {
                            let id = env.message_id.clone();
                            if handle.send_envelope(env).is_ok() {
                                link.ack(&id);
                            }
                            let _ = kick.unbounded_send(());
                        }
                        Ok(Inbound::Acked { watched: true, .. }) | Ok(Inbound::Refused { .. }) => {
                            let _ = kick.unbounded_send(());
                        }
                        _ => {}
                    },
                    Ev::Closed => break,
                }
                if closed.get() {
                    break;
                }
            }
            link.disconnected(generation);
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onclose(None);
            ws.set_onerror(None);
            let _ = ws.close();
            socket.borrow_mut().take();
            drop((on_open, on_message, on_close, on_error));
            if closed.get() {
                break;
            }
            // a refused credential waits the longest delay before asking the provider again
            let wait = if link.refusal().is_some() {
                MAX_BACKOFF_MS
            } else {
                backoff
            };
            TimeoutSleep
                .sleep_until(js_sys::Date::now() as i64 + wait)
                .await;
            backoff = (backoff * 2).min(MAX_BACKOFF_MS);
        }
    });
    c
}
