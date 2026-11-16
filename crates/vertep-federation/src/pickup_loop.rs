//! The background pickup of a relay inbox: live where the relay offers a session, polling
//! where it does not or while the session is down.
//!
//! A [`PickupLoop`] runs one thread per inbox. For each pickup binding it tries to open a live
//! session ([`crate::RelayInbox::go_live_at`]). While the session is open the relay pushes and
//! nothing is polled (unless [`PickupLoopConfig::live_poll_interval`] asks for a safety poll).
//! When the session cannot be opened, or drops, the binding falls back to polling every
//! [`PickupLoopConfig::poll_interval`] and retries the session with exponential backoff. Each
//! time a session opens, the binding polls once, so what arrived while it was reconnecting is
//! picked up even from a relay that does not replay its queue on a new session.
//!
//! Nothing here acknowledges or deduplicates: a pushed or polled message is placed in the
//! actor's mailbox and acknowledged after the journal append as always, a copy fetched twice
//! is attached to the row that already holds it, and a copy redelivered after its
//! acknowledgement is dropped by the journal as a duplicate message id.

use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::inbox::RelayInbox;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How often a [`PickupLoop`] polls and reconnects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickupLoopConfig {
    /// Between polls of a binding without a live session.
    pub poll_interval: Duration,
    /// Between safety polls of a binding with a live session; `None` polls only on connect.
    pub live_poll_interval: Option<Duration>,
    /// The first wait before reconnecting a session that failed or dropped.
    pub reconnect_min: Duration,
    /// The longest wait between reconnect attempts.
    pub reconnect_max: Duration,
    /// How often sessions are checked (and the loop wakes to poll).
    pub check_interval: Duration,
}

impl Default for PickupLoopConfig {
    fn default() -> Self {
        PickupLoopConfig {
            poll_interval: Duration::from_secs(5),
            live_poll_interval: None,
            reconnect_min: Duration::from_millis(500),
            reconnect_max: Duration::from_secs(30),
            check_interval: Duration::from_millis(50),
        }
    }
}

/// How a binding is picked up right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickupMode {
    /// The relay pushes on an open live session.
    Live,
    /// No session: the binding is polled.
    Polling,
}

/// What a [`PickupLoop`] did for one binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingStatus {
    /// The binding's name.
    pub name: String,
    /// The current mode.
    pub mode: PickupMode,
    /// Polls made (fetch requests over the polling path).
    pub polls: u64,
    /// Live sessions opened.
    pub connects: u64,
    /// Attempts to open a session that failed.
    pub failed_connects: u64,
    /// Open sessions that dropped.
    pub drops: u64,
}

struct Binding {
    status: BindingStatus,
    next_poll: Instant,
    next_connect: Instant,
    backoff: Duration,
}

struct Shared {
    inbox: RelayInbox,
    config: PickupLoopConfig,
    bindings: Mutex<Vec<Binding>>,
    stop: Mutex<bool>,
    wake: Condvar,
}

impl Shared {
    fn poll(&self, i: usize, b: &mut Binding, now: Instant) {
        self.inbox.pickup_at(i);
        b.status.polls += 1;
        b.next_poll = now + self.config.poll_interval;
    }

    fn tick(&self) {
        let n = lock(&self.bindings).len();
        for i in 0..n {
            // Only this thread changes the state, so a copy is worked on without the lock while
            // the network is in use.
            let Some(mut b) = lock(&self.bindings).get(i).map(|b| Binding {
                status: b.status.clone(),
                next_poll: b.next_poll,
                next_connect: b.next_connect,
                backoff: b.backoff,
            }) else {
                continue;
            };
            let now = Instant::now();
            if b.status.mode == PickupMode::Live {
                if !self.inbox.live_open_at(i) {
                    // The socket dropped: fall back to polling at once.
                    b.status.mode = PickupMode::Polling;
                    b.status.drops += 1;
                    b.backoff = self.config.reconnect_min;
                    b.next_connect = now + b.backoff;
                    self.poll(i, &mut b, now);
                } else if self.config.live_poll_interval.is_some() && now >= b.next_poll {
                    self.poll(i, &mut b, now);
                    if let Some(every) = self.config.live_poll_interval {
                        b.next_poll = now + every;
                    }
                }
            }
            if b.status.mode == PickupMode::Polling {
                if now >= b.next_connect {
                    if self.inbox.go_live_at(i) {
                        b.status.mode = PickupMode::Live;
                        b.status.connects += 1;
                        b.backoff = self.config.reconnect_min;
                        // Resume: what arrived while there was no session.
                        self.poll(i, &mut b, now);
                        if let Some(every) = self.config.live_poll_interval {
                            b.next_poll = now + every;
                        }
                    } else {
                        b.status.failed_connects += 1;
                        b.next_connect = now + b.backoff;
                        b.backoff = (b.backoff * 2).min(self.config.reconnect_max);
                    }
                }
                if b.status.mode == PickupMode::Polling && now >= b.next_poll {
                    self.poll(i, &mut b, now);
                }
            }
            if let Some(slot) = lock(&self.bindings).get_mut(i) {
                *slot = b;
            }
        }
    }
}

/// Picks up a relay inbox in the background: live sessions where possible, polling otherwise.
/// Stops when dropped.
// @tg: implements:: [[openspec:relay-mailboxes#WebSocket Live Delivery]] {note: "fallback to polling and resumption"}
pub struct PickupLoop {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for PickupLoop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PickupLoop")
            .field("config", &self.shared.config)
            .field("bindings", &self.status())
            .finish()
    }
}

impl PickupLoop {
    /// Starts picking up `inbox` per `config`.
    ///
    /// # Errors
    ///
    /// The I/O error when the thread cannot be started.
    pub fn spawn(inbox: RelayInbox, config: PickupLoopConfig) -> std::io::Result<Self> {
        let now = Instant::now();
        let bindings = inbox
            .binding_names()
            .into_iter()
            .map(|name| Binding {
                status: BindingStatus {
                    name,
                    mode: PickupMode::Polling,
                    polls: 0,
                    connects: 0,
                    failed_connects: 0,
                    drops: 0,
                },
                next_poll: now,
                next_connect: now,
                backoff: config.reconnect_min,
            })
            .collect();
        let shared = Arc::new(Shared {
            inbox,
            config,
            bindings: Mutex::new(bindings),
            stop: Mutex::new(false),
            wake: Condvar::new(),
        });
        let s = shared.clone();
        let thread = std::thread::Builder::new()
            .name("vt-pickup".into())
            .spawn(move || {
                loop {
                    s.tick();
                    let stop = lock(&s.stop);
                    if *stop {
                        break;
                    }
                    let (stop, _) = s
                        .wake
                        .wait_timeout(stop, s.config.check_interval)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if *stop {
                        break;
                    }
                }
            })?;
        Ok(PickupLoop {
            shared,
            thread: Some(thread),
        })
    }

    /// The status of every binding, in binding order.
    pub fn status(&self) -> Vec<BindingStatus> {
        lock(&self.shared.bindings)
            .iter()
            .map(|b| b.status.clone())
            .collect()
    }

    /// The mode of the binding `name`.
    pub fn mode(&self, name: &str) -> Option<PickupMode> {
        lock(&self.shared.bindings)
            .iter()
            .find(|b| b.status.name == name)
            .map(|b| b.status.mode)
    }

    /// Stops the loop and waits for its thread.
    pub fn stop(mut self) {
        self.halt();
    }

    fn halt(&mut self) {
        *lock(&self.shared.stop) = true;
        self.shared.wake.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PickupLoop {
    fn drop(&mut self) {
        self.halt();
    }
}
