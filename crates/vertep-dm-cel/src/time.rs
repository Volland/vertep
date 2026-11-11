//! CEL `google.protobuf.Timestamp` and `google.protobuf.Duration` values as integer
//! nanoseconds, with RFC 3339 and duration-string parsing and formatting.
//!
//! There is no clock: a timestamp only ever comes from a literal string, an integer or chart
//! data. Time zones are `UTC` or fixed offsets (`+05:30`); named IANA zones are not available,
//! because a time zone database would make results depend on its version.

use alloc::format;
use alloc::string::String;

const NANOS: i128 = 1_000_000_000;
/// `0001-01-01T00:00:00Z` in seconds since the Unix epoch.
const TS_MIN_SECS: i128 = -62_135_596_800;
/// `9999-12-31T23:59:59Z` in seconds since the Unix epoch.
const TS_MAX_SECS: i128 = 253_402_300_799;
/// The largest duration magnitude in seconds (about 10000 years).
const DUR_MAX_SECS: i128 = 315_576_000_000;

/// An instant, in nanoseconds since the Unix epoch, between years 1 and 9999.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i128);

/// A signed span of time in nanoseconds, at most about 10000 years.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(i128);

impl Timestamp {
    /// A timestamp from nanoseconds since the epoch, when in range.
    pub fn from_nanos(n: i128) -> Option<Self> {
        let secs = n.div_euclid(NANOS);
        (TS_MIN_SECS..=TS_MAX_SECS)
            .contains(&secs)
            .then_some(Timestamp(n))
    }

    /// A timestamp from whole seconds since the epoch.
    pub fn from_secs(s: i64) -> Option<Self> {
        Timestamp::from_nanos(i128::from(s) * NANOS)
    }

    /// Nanoseconds since the epoch.
    pub fn nanos(&self) -> i128 {
        self.0
    }

    /// Whole seconds since the epoch, rounded toward negative infinity.
    pub fn secs(&self) -> i64 {
        // In range by construction.
        i64::try_from(self.0.div_euclid(NANOS)).unwrap_or(i64::MAX)
    }

    /// Parse an RFC 3339 timestamp with an explicit offset.
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() < 20 {
            return None;
        }
        let num = |r: core::ops::Range<usize>| -> Option<i64> {
            let t = s.get(r)?;
            if !t.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            t.parse().ok()
        };
        let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        if b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') {
            return None;
        }
        let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
        if b[13] != b':' || b[16] != b':' {
            return None;
        }
        if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 {
            return None;
        }
        if sec > 59 {
            return None;
        }
        let mut i = 19;
        let mut frac: i128 = 0;
        if b.get(i) == Some(&b'.') {
            i += 1;
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let digits = &s[start..i];
            if digits.is_empty() || digits.len() > 9 {
                return None;
            }
            frac = digits.parse::<i128>().ok()? * 10i128.pow(9 - digits.len() as u32);
        }
        let offset = parse_offset(s.get(i..)?)?;
        let days = days_from_civil(y, mo, d);
        let secs =
            i128::from(days) * 86_400 + i128::from(h * 3600 + mi * 60 + sec) - i128::from(offset);
        Timestamp::from_nanos(secs * NANOS + frac)
    }

    /// The broken-down civil time at a fixed offset in seconds.
    pub fn civil(&self, offset_secs: i64) -> Civil {
        let local = self.0 + i128::from(offset_secs) * NANOS;
        let secs = local.div_euclid(NANOS);
        let nanos = local.rem_euclid(NANOS);
        let days = i64::try_from(secs.div_euclid(86_400)).unwrap_or(0);
        let sod = i64::try_from(secs.rem_euclid(86_400)).unwrap_or(0);
        let (y, m, d) = civil_from_days(days);
        Civil {
            year: y,
            month: m,
            day: d,
            hour: sod / 3600,
            minute: sod % 3600 / 60,
            second: sod % 60,
            nanos: i64::try_from(nanos).unwrap_or(0),
            weekday: (days + 4).rem_euclid(7),
            yday: days - days_from_civil(y, 1, 1),
        }
    }

    /// RFC 3339 in UTC with the fraction trimmed of trailing zeros.
    pub fn format(&self) -> String {
        let c = self.civil(0);
        let frac = trim_fraction(c.nanos);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{frac}Z",
            c.year, c.month, c.day, c.hour, c.minute, c.second
        )
    }

    /// `self + d`, when in range.
    pub fn plus(self, d: Duration) -> Option<Timestamp> {
        Timestamp::from_nanos(self.0 + d.0)
    }

    /// `self - other`, when in range.
    pub fn since(self, other: Timestamp) -> Option<Duration> {
        Duration::from_nanos(self.0 - other.0)
    }
}

/// Broken-down civil time.
#[derive(Clone, Copy, Debug)]
pub struct Civil {
    /// Year.
    pub year: i64,
    /// Month, 1 to 12.
    pub month: i64,
    /// Day of month, 1 to 31.
    pub day: i64,
    /// Hour.
    pub hour: i64,
    /// Minute.
    pub minute: i64,
    /// Second.
    pub second: i64,
    /// Nanoseconds within the second.
    pub nanos: i64,
    /// Day of week, 0 for Sunday.
    pub weekday: i64,
    /// Day of year, 0-based.
    pub yday: i64,
}

impl Duration {
    /// A duration from nanoseconds, when in range.
    pub fn from_nanos(n: i128) -> Option<Self> {
        (n.abs() < (DUR_MAX_SECS + 1) * NANOS).then_some(Duration(n))
    }

    /// Nanoseconds.
    pub fn nanos(&self) -> i128 {
        self.0
    }

    /// Parse a duration string such as `1h30m`, `1.5s` or `-250ms`.
    pub fn parse(s: &str) -> Option<Self> {
        let (neg, mut rest) = match s.as_bytes().first() {
            Some(b'-') => (true, &s[1..]),
            Some(b'+') => (false, &s[1..]),
            _ => (false, s),
        };
        if rest == "0" {
            return Some(Duration(0));
        }
        if rest.is_empty() {
            return None;
        }
        let mut total: i128 = 0;
        while !rest.is_empty() {
            let int_end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let int_part = &rest[..int_end];
            rest = &rest[int_end..];
            let mut frac_part = "";
            if let Some(r) = rest.strip_prefix('.') {
                let fe = r.find(|c: char| !c.is_ascii_digit()).unwrap_or(r.len());
                frac_part = &r[..fe];
                rest = &r[fe..];
            }
            if int_part.is_empty() && frac_part.is_empty() {
                return None;
            }
            let units: [(&str, i128); 7] = [
                ("ns", 1),
                ("us", 1_000),
                ("\u{b5}s", 1_000),
                ("ms", 1_000_000),
                ("s", NANOS),
                ("m", 60 * NANOS),
                ("h", 3600 * NANOS),
            ];
            let (unit, scale) = units.iter().find(|(u, _)| rest.starts_with(u))?;
            rest = &rest[unit.len()..];
            if int_part.len() > 15 || frac_part.len() > 18 {
                return None;
            }
            let i: i128 = if int_part.is_empty() {
                0
            } else {
                int_part.parse().ok()?
            };
            let mut v = i.checked_mul(*scale)?;
            if !frac_part.is_empty() {
                let f: i128 = frac_part.parse().ok()?;
                v += f * scale / 10i128.pow(frac_part.len() as u32);
            }
            total = total.checked_add(v)?;
        }
        Duration::from_nanos(if neg { -total } else { total })
    }

    /// Seconds with a trimmed fraction and an `s` suffix, such as `5400s` or `-1.5s`.
    pub fn format(&self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let a = self.0.abs();
        let frac = trim_fraction(i64::try_from(a % NANOS).unwrap_or(0));
        format!("{sign}{}{frac}s", a / NANOS)
    }

    /// `self + o`, when in range.
    pub fn plus(self, o: Duration) -> Option<Duration> {
        Duration::from_nanos(self.0 + o.0)
    }
}

fn trim_fraction(nanos: i64) -> String {
    if nanos == 0 {
        return String::new();
    }
    let s = format!("{nanos:09}");
    format!(".{}", s.trim_end_matches('0'))
}

/// Parse `Z`, `z` or `±HH:MM` into seconds east of UTC.
fn parse_offset(s: &str) -> Option<i64> {
    if s == "Z" || s == "z" {
        return Some(0);
    }
    let b = s.as_bytes();
    if b.len() != 6 || b[3] != b':' {
        return None;
    }
    let sign = match b[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let h: i64 = s.get(1..3)?.parse().ok()?;
    let m: i64 = s.get(4..6)?.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}

/// Parse a CEL time zone argument: `UTC`, `Z` or a fixed offset `±HH:MM`.
pub fn parse_zone(s: &str) -> Option<i64> {
    if s.eq_ignore_ascii_case("utc") {
        return Some(0);
    }
    parse_offset(s)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_round_trip() {
        let t = Timestamp::parse("2024-02-29T23:30:00.250+01:00").unwrap();
        assert_eq!(t.format(), "2024-02-29T22:30:00.25Z");
        assert_eq!(Timestamp::parse("1970-01-01T00:00:00Z").unwrap().nanos(), 0);
        assert!(Timestamp::parse("2023-02-29T00:00:00Z").is_none());
        assert!(Timestamp::parse("2023-01-01T00:00:00").is_none());
        let c = Timestamp::parse("2024-03-01T00:00:00Z").unwrap().civil(0);
        assert_eq!(
            (c.year, c.month, c.day, c.weekday, c.yday),
            (2024, 3, 1, 5, 60)
        );
    }

    #[test]
    fn durations_round_trip() {
        assert_eq!(Duration::parse("1h30m").unwrap().format(), "5400s");
        assert_eq!(Duration::parse("-1.5s").unwrap().format(), "-1.5s");
        assert_eq!(Duration::parse("250ms").unwrap().nanos(), 250_000_000);
        assert!(Duration::parse("1x").is_none());
        assert!(Duration::parse("").is_none());
    }
}
