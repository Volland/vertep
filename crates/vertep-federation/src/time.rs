//! Instants and the injectable clock used for cache lifetimes, grant expiry, credential validity
//! and key retirement.
//!
//! Every time-dependent decision in this crate takes a [`Timestamp`] from a [`Clock`] the caller
//! supplies, so tests are deterministic and the runtime can pass the instant it journals.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// An instant as milliseconds since the Unix epoch (UTC).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// The instant `secs` seconds after the epoch.
    pub const fn from_secs(secs: i64) -> Self {
        Timestamp(secs * 1000)
    }

    /// Milliseconds since the epoch.
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// This instant moved forward by `d` (saturating).
    pub fn plus(self, d: Duration) -> Self {
        let ms = i64::try_from(d.as_millis()).unwrap_or(i64::MAX);
        Timestamp(self.0.saturating_add(ms))
    }

    /// The current system time.
    pub fn now() -> Self {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => Timestamp(i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
            Err(e) => Timestamp(-i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX)),
        }
    }

    /// The RFC 3339 / `xsd:dateTime` form in UTC, with milliseconds only when non-zero.
    pub fn to_rfc3339(self) -> String {
        let secs = self.0.div_euclid(1000);
        let ms = self.0.rem_euclid(1000);
        let days = secs.div_euclid(86_400);
        let sod = secs.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
        if ms == 0 {
            format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
        } else {
            format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{ms:03}Z")
        }
    }

    /// Parses an RFC 3339 date-time (`YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`).
    pub fn parse_rfc3339(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
            return None;
        }
        if b[13] != b':' || b[16] != b':' {
            return None;
        }
        let num = |r: std::ops::Range<usize>| -> Option<i64> {
            let t = s.get(r)?;
            if !t.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            t.parse().ok()
        };
        let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
        if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
            return None;
        }
        let mut i = 19;
        let mut ms = 0i64;
        if b.get(i) == Some(&b'.') {
            i += 1;
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return None;
            }
            let frac = &s[start..i.min(start + 3)];
            ms = frac.parse::<i64>().ok()? * 10i64.pow(3 - frac.len() as u32);
        }
        let offset_secs = match b.get(i)? {
            b'Z' | b'z' if i + 1 == b.len() => 0,
            c @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
                let oh = num(i + 1..i + 3)?;
                let om = num(i + 4..i + 6)?;
                let o = oh * 3600 + om * 60;
                if *c == b'+' { o } else { -o }
            }
            _ => return None,
        };
        let days = days_from_civil(y, mo, d);
        let secs = days * 86_400 + h * 3600 + mi * 60 + se - offset_secs;
        Some(Timestamp(secs * 1000 + ms))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339())
    }
}

// Howard Hinnant's civil calendar algorithms (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The source of the current instant.
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> Timestamp;
}

/// The system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock that only moves when told to, for tests and replay.
#[derive(Debug, Default)]
pub struct ManualClock(std::sync::atomic::AtomicI64);

impl ManualClock {
    /// A clock showing `t`.
    pub fn new(t: Timestamp) -> Self {
        ManualClock(std::sync::atomic::AtomicI64::new(t.0))
    }

    /// Sets the time to `t`.
    pub fn set(&self, t: Timestamp) {
        self.0.store(t.0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Moves the time forward by `d`.
    pub fn advance(&self, d: Duration) {
        let now = self.now();
        self.set(now.plus(d));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip() {
        for s in [
            "1970-01-01T00:00:00Z",
            "2026-11-11T10:20:30Z",
            "2000-02-29T23:59:59.250Z",
        ] {
            let t = Timestamp::parse_rfc3339(s).expect("parses");
            assert_eq!(t.to_rfc3339(), s);
        }
        let a = Timestamp::parse_rfc3339("2026-01-01T02:00:00+02:00").expect("offset");
        assert_eq!(a.to_rfc3339(), "2026-01-01T00:00:00Z");
        assert!(Timestamp::parse_rfc3339("2026-13-01T00:00:00Z").is_none());
        assert!(Timestamp::parse_rfc3339("not a date").is_none());
    }
}
