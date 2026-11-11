//! Monotonic time for leases and failure detection, injectable for deterministic tests.

use core::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// An instant on a monotonic clock, in milliseconds since the clock's origin.
///
/// Instants of different clocks are not comparable: a lease's expiry is measured on the lease
/// authority's clock, and a holder converts it to its own clock by the time it sent the
/// request (see [`crate::lease::HeldLeases`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Millis(pub u64);

impl Millis {
    /// This instant plus `d`, saturating.
    pub fn after(self, d: Duration) -> Millis {
        Millis(self.0.saturating_add(duration_ms(d)))
    }

    /// This instant minus `d`, saturating at the origin.
    pub fn before(self, d: Duration) -> Millis {
        Millis(self.0.saturating_sub(duration_ms(d)))
    }

    /// The time from `earlier` to this instant, zero when `earlier` is later.
    pub fn since(self, earlier: Millis) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

impl fmt::Display for Millis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// `d` in whole milliseconds, saturating.
pub fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// A monotonic clock.
pub trait Clock: Send + Sync + fmt::Debug {
    /// The current instant.
    fn now(&self) -> Millis;
}

impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> Millis {
        (**self).now()
    }
}

/// The process's monotonic clock (`std::time::Instant`), with its origin at construction.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    /// A clock starting at zero now.
    pub fn new() -> Self {
        MonotonicClock {
            origin: Instant::now(),
        }
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for MonotonicClock {
    fn now(&self) -> Millis {
        Millis(duration_ms(self.origin.elapsed()))
    }
}

/// A clock that moves only when told to, shared by its clones. Tests and simulations give
/// every node its own manual clock, so pauses and skew are just clocks that are not advanced.
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    /// A clock at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock at `ms`.
    pub fn at(ms: u64) -> Self {
        ManualClock(Arc::new(AtomicU64::new(ms)))
    }

    /// Moves the clock forward by `d`.
    pub fn advance(&self, d: Duration) {
        self.0.fetch_add(duration_ms(d), Ordering::SeqCst);
    }

    /// Moves the clock forward by `ms` milliseconds.
    pub fn advance_ms(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }

    /// Sets the clock to `ms`; a monotonic clock never goes back, so an earlier value is
    /// ignored.
    pub fn set(&self, ms: u64) {
        self.0.fetch_max(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Millis {
        Millis(self.0.load(Ordering::SeqCst))
    }
}
