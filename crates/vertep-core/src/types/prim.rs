//! Lexical rules of the primitive types: 64-bit integers, decimals, RFC 3339 datetimes and
//! RFC 3987 IRIs.

use alloc::format;
use alloc::string::String;

/// Why a string is not an RFC 3339 datetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatetimeError {
    /// Date and time are well formed but no offset (`Z` or `+hh:mm`) follows.
    MissingOffset,
    /// The text is not an RFC 3339 `date-time`; the string names the offending part.
    Invalid(&'static str),
}

/// A parsed RFC 3339 datetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datetime {
    /// Year, 0 to 9999.
    pub year: u32,
    /// Month, 1 to 12.
    pub month: u32,
    /// Day of month.
    pub day: u32,
    /// Hour, 0 to 23.
    pub hour: u32,
    /// Minute, 0 to 59.
    pub minute: u32,
    /// Second, 0 to 60 (leap second).
    pub second: u32,
    /// Fractional seconds digits, without the dot; empty when absent.
    pub fraction: String,
    /// Offset from UTC in minutes.
    pub offset_minutes: i32,
}

fn digits(b: &[u8], at: usize, n: usize) -> Option<u32> {
    let s = b.get(at..at + n)?;
    if !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(s.iter().fold(0, |acc, d| acc * 10 + u32::from(d - b'0')))
}

fn is_leap(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        _ => 28,
    }
}

/// Parse an RFC 3339 `date-time` (`2026-11-11T10:00:00+02:00`). The offset is required.
pub fn parse_datetime(s: &str) -> Result<Datetime, DatetimeError> {
    use DatetimeError::Invalid;
    let b = s.as_bytes();
    let year = digits(b, 0, 4).ok_or(Invalid("year"))?;
    if b.get(4) != Some(&b'-') {
        return Err(Invalid("date separator"));
    }
    let month = digits(b, 5, 2).ok_or(Invalid("month"))?;
    if b.get(7) != Some(&b'-') {
        return Err(Invalid("date separator"));
    }
    let day = digits(b, 8, 2).ok_or(Invalid("day"))?;
    if !(1..=12).contains(&month) {
        return Err(Invalid("month"));
    }
    if day == 0 || day > days_in_month(year, month) {
        return Err(Invalid("day"));
    }
    if !matches!(b.get(10), Some(b'T' | b't')) {
        return Err(Invalid("time separator `T`"));
    }
    let hour = digits(b, 11, 2).ok_or(Invalid("hour"))?;
    let minute = (b.get(13) == Some(&b':'))
        .then(|| digits(b, 14, 2))
        .flatten()
        .ok_or(Invalid("minute"))?;
    let second = (b.get(16) == Some(&b':'))
        .then(|| digits(b, 17, 2))
        .flatten()
        .ok_or(Invalid("second"))?;
    if hour > 23 {
        return Err(Invalid("hour"));
    }
    if minute > 59 {
        return Err(Invalid("minute"));
    }
    if second > 60 {
        return Err(Invalid("second"));
    }
    let mut i = 19;
    let mut fraction = String::new();
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return Err(Invalid("fractional seconds"));
        }
        fraction.push_str(&s[start..i]);
    }
    let offset_minutes = match b.get(i) {
        None => return Err(DatetimeError::MissingOffset),
        Some(b'Z' | b'z') => {
            i += 1;
            0
        }
        Some(sign @ (b'+' | b'-')) => {
            let oh = digits(b, i + 1, 2).ok_or(Invalid("offset"))?;
            let om = (b.get(i + 3) == Some(&b':'))
                .then(|| digits(b, i + 4, 2))
                .flatten()
                .ok_or(Invalid("offset"))?;
            if oh > 23 || om > 59 {
                return Err(Invalid("offset"));
            }
            i += 6;
            let m = (oh * 60 + om) as i32;
            if *sign == b'-' { -m } else { m }
        }
        Some(_) => return Err(Invalid("offset")),
    };
    if i != b.len() {
        return Err(Invalid("trailing text"));
    }
    Ok(Datetime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        fraction,
        offset_minutes,
    })
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Civil date of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

impl Datetime {
    /// The same instant in UTC, written `YYYY-MM-DDTHH:MM:SS[.fff]Z`. Leap seconds are kept as
    /// second 60. Returns `None` when the UTC year leaves 0000 to 9999.
    pub fn to_utc_string(&self) -> Option<String> {
        let days = days_from_civil(
            i64::from(self.year),
            i64::from(self.month),
            i64::from(self.day),
        );
        let minutes = days * 1440 + i64::from(self.hour) * 60 + i64::from(self.minute)
            - i64::from(self.offset_minutes);
        let (d, mins) = (minutes.div_euclid(1440), minutes.rem_euclid(1440));
        let (y, m, dd) = civil_from_days(d);
        if !(0..=9999).contains(&y) {
            return None;
        }
        let frac = if self.fraction.is_empty() {
            String::new()
        } else {
            format!(".{}", self.fraction)
        };
        Some(format!(
            "{y:04}-{m:02}-{dd:02}T{:02}:{:02}:{:02}{frac}Z",
            mins / 60,
            mins % 60,
            self.second
        ))
    }
}

/// Why a string is not an absolute IRI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IriError {
    /// The IRI has no scheme: it is a relative reference.
    Relative,
    /// The IRI contains a character that IRIs exclude, or a bad percent-encoding.
    InvalidCharacter,
}

/// Check that `s` is an IRI with a scheme (RFC 3987 `IRI`): `scheme ":" ...`, without spaces,
/// control characters, `<>"{}|\^` and backquote, and with well-formed percent-encodings. A
/// fragment is allowed.
pub fn check_iri(s: &str) -> Result<(), IriError> {
    let scheme_len = s.find(':').ok_or(IriError::Relative)?;
    let scheme = &s[..scheme_len];
    let mut cs = scheme.chars();
    let scheme_ok = matches!(cs.next(), Some(c) if c.is_ascii_alphabetic())
        && cs.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    if !scheme_ok {
        return Err(IriError::Relative);
    }
    let b = s.as_bytes();
    let mut i = scheme_len + 1;
    while i < b.len() {
        let c = b[i];
        if c == b'%' {
            let ok = b.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
            if !ok {
                return Err(IriError::InvalidCharacter);
            }
            i += 3;
            continue;
        }
        if c <= 0x20 || c == 0x7f || b"<>\"{}|\\^`".contains(&c) {
            return Err(IriError::InvalidCharacter);
        }
        i += 1;
    }
    Ok(())
}

/// Whether `s` is a decimal literal: `-?[0-9]+(\.[0-9]+)?`.
pub fn is_decimal(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    all_digits(int) && frac.is_none_or(all_digits)
}

/// The `i64` a finite, integral `f64` denotes, or `None` when it has a fraction or is outside
/// the signed 64-bit range.
pub fn f64_to_i64(f: f64) -> Option<i64> {
    // 2^63 is exactly representable; every f64 below it in magnitude converts exactly.
    const LIMIT: f64 = 9_223_372_036_854_775_808.0;
    if !f.is_finite() || !(-LIMIT..LIMIT).contains(&f) {
        return None;
    }
    let i = f as i64;
    (i as f64 == f).then_some(i)
}
