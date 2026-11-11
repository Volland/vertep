//! The shell traits: the only way the runtime spawns work, reads time or waits.
//!
//! The core (`vertep-core`) reads no clock and spawns nothing. The runtime shell does both, but
//! only through [`Spawn`], [`Clock`] and [`Sleep`], so that the same runtime runs on tokio
//! ([`Shell::tokio`]), on a deterministic virtual clock in tests ([`Shell::virtual_time`]) and,
//! later, in the browser.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A boxed, sendable future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Runs a future to completion in the background.
pub trait Spawn: Send + Sync {
    /// Starts `fut`; its output is discarded.
    fn spawn(&self, fut: BoxFuture<()>);
}

/// The time source of the shell. Only the shell reads it: a macrostep sees the time stamped on
/// its input when the input was dequeued, which is journaled.
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}

/// Waits until a point in time of the shell's [`Clock`].
pub trait Sleep: Send + Sync {
    /// A future that completes once the clock reads at least `at_ms`.
    fn sleep_until(&self, at_ms: i64) -> BoxFuture<()>;
}

/// The three shell services a runtime node uses.
#[derive(Clone)]
pub struct Shell {
    /// Background work: function actors, stream actors and stateless workers.
    pub spawn: Arc<dyn Spawn>,
    /// The time source.
    pub clock: Arc<dyn Clock>,
    /// Waiting for timers, idle timeouts and ask timeouts.
    pub sleep: Arc<dyn Sleep>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("now_ms", &self.clock.now_ms())
            .finish_non_exhaustive()
    }
}

impl Shell {
    /// The tokio shell: `tokio::spawn`, the system clock and `tokio::time::sleep`. Spawning
    /// needs a running tokio runtime.
    pub fn tokio() -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        Shell {
            spawn: Arc::new(TokioSpawn),
            sleep: Arc::new(TokioSleep {
                clock: clock.clone(),
            }),
            clock,
        }
    }

    /// A deterministic shell for tests: tokio spawning and a [`VirtualClock`] starting at
    /// `start_ms` that moves only when advanced.
    pub fn virtual_time(start_ms: i64) -> (Self, VirtualClock) {
        let clock = VirtualClock::new(start_ms);
        let shell = Shell {
            spawn: Arc::new(TokioSpawn),
            clock: Arc::new(clock.clone()),
            sleep: Arc::new(clock.clone()),
        };
        (shell, clock)
    }
}

/// Spawns on the current tokio runtime.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokioSpawn;

impl Spawn for TokioSpawn {
    fn spawn(&self, fut: BoxFuture<()>) {
        tokio::spawn(fut);
    }
}

/// The system wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// Sleeps with `tokio::time::sleep` until a time of the given clock.
#[derive(Clone)]
pub struct TokioSleep {
    clock: Arc<dyn Clock>,
}

impl TokioSleep {
    /// A sleeper measuring against `clock`.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        TokioSleep { clock }
    }
}

impl std::fmt::Debug for TokioSleep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioSleep").finish_non_exhaustive()
    }
}

impl Sleep for TokioSleep {
    fn sleep_until(&self, at_ms: i64) -> BoxFuture<()> {
        let wait = at_ms.saturating_sub(self.clock.now_ms()).max(0);
        Box::pin(tokio::time::sleep(Duration::from_millis(
            u64::try_from(wait).unwrap_or(0),
        )))
    }
}

#[derive(Default)]
struct VirtualInner {
    now: i64,
    waiters: Vec<(i64, Waker)>,
}

/// A clock that moves only when [`VirtualClock::advance`] or [`VirtualClock::set`] is called,
/// and wakes the sleepers whose time has come. Cloning shares the clock.
#[derive(Clone, Default)]
pub struct VirtualClock {
    inner: Arc<Mutex<VirtualInner>>,
}

impl std::fmt::Debug for VirtualClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualClock")
            .field("now", &self.now_ms())
            .finish()
    }
}

impl VirtualClock {
    /// A clock reading `start_ms`.
    pub fn new(start_ms: i64) -> Self {
        VirtualClock {
            inner: Arc::new(Mutex::new(VirtualInner {
                now: start_ms,
                waiters: Vec::new(),
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VirtualInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Moves the clock forward by `d`.
    pub fn advance(&self, d: Duration) {
        let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        let now = self.now_ms().saturating_add(ms);
        self.set(now);
    }

    /// Sets the clock to `at_ms` (never backwards) and wakes due sleepers.
    pub fn set(&self, at_ms: i64) {
        let due: Vec<Waker> = {
            let mut g = self.lock();
            g.now = g.now.max(at_ms);
            let now = g.now;
            let (ready, wait): (Vec<_>, Vec<_>) = g.waiters.drain(..).partition(|(t, _)| *t <= now);
            g.waiters = wait;
            ready.into_iter().map(|(_, w)| w).collect()
        };
        for w in due {
            w.wake();
        }
    }
}

impl Clock for VirtualClock {
    fn now_ms(&self) -> i64 {
        self.lock().now
    }
}

impl Sleep for VirtualClock {
    fn sleep_until(&self, at_ms: i64) -> BoxFuture<()> {
        Box::pin(VirtualSleep {
            clock: self.clone(),
            at: at_ms,
        })
    }
}

struct VirtualSleep {
    clock: VirtualClock,
    at: i64,
}

impl Future for VirtualSleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut g = self.clock.lock();
        if g.now >= self.at {
            Poll::Ready(())
        } else {
            g.waiters.push((self.at, cx.waker().clone()));
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_clock_moves_only_when_advanced() {
        let c = VirtualClock::new(1_000);
        assert_eq!(c.now_ms(), 1_000);
        c.advance(Duration::from_millis(250));
        assert_eq!(c.now_ms(), 1_250);
        c.set(100);
        assert_eq!(c.now_ms(), 1_250, "never backwards");
    }

    #[tokio::test]
    async fn virtual_sleep_completes_after_advance() {
        let c = VirtualClock::new(0);
        let s = c.sleep_until(50);
        let h = tokio::spawn(s);
        tokio::task::yield_now().await;
        assert!(!h.is_finished());
        c.advance(Duration::from_millis(50));
        h.await.unwrap();
    }
}
