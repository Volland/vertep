//! Exact decimal numbers for the CEL `decimal` extension type.
//!
//! A [`Decimal`] is a signed integer mantissa of at most [`MAX_DIGITS`] decimal digits and a
//! scale of at most [`MAX_SCALE`] fractional digits, kept normalized (no trailing fractional
//! zeros), so equal numbers have equal representations. Addition, subtraction, multiplication
//! and remainder are exact or fail; division is exact whenever the quotient fits and is
//! otherwise rounded half to even at [`MAX_DIGITS`] significant digits. Every operation is
//! integer arithmetic, so results are identical on every target.

use alloc::format;
use alloc::string::String;
use core::cmp::Ordering;
use core::fmt;

/// The largest number of significant digits (IEEE 754 decimal128 precision).
pub const MAX_DIGITS: u32 = 34;
/// The largest number of fractional digits.
pub const MAX_SCALE: u32 = 34;

const LIMIT: u128 = 10u128.pow(MAX_DIGITS);

/// Why a decimal operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalError {
    /// The result needs more than [`MAX_DIGITS`] digits or [`MAX_SCALE`] fractional digits.
    Overflow,
    /// Division or remainder by zero.
    DivisionByZero,
    /// The text is not a decimal number.
    Syntax,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DecimalError::Overflow => {
                "decimal out of range: at most 34 significant and 34 fractional digits"
            }
            DecimalError::DivisionByZero => "division by zero",
            DecimalError::Syntax => "invalid decimal syntax",
        })
    }
}

/// An exact decimal number.
// @tg: implements:: [[openspec:cel-datamodel#Deterministic Evaluation]]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Decimal {
    mantissa: i128,
    scale: u32,
}

fn pow10(n: u32) -> Option<i128> {
    10i128.checked_pow(n)
}

impl Decimal {
    /// Zero.
    pub const ZERO: Decimal = Decimal {
        mantissa: 0,
        scale: 0,
    };

    /// `mantissa × 10^-scale`, normalized; fails when it does not fit.
    pub fn new(mantissa: i128, scale: u32) -> Result<Self, DecimalError> {
        let (mut m, mut s) = (mantissa, scale);
        while s > 0 && m % 10 == 0 {
            m /= 10;
            s -= 1;
        }
        if m == 0 {
            return Ok(Decimal::ZERO);
        }
        if m.unsigned_abs() >= LIMIT || s > MAX_SCALE {
            return Err(DecimalError::Overflow);
        }
        Ok(Decimal {
            mantissa: m,
            scale: s,
        })
    }

    /// The exact value of an `i64`.
    pub fn from_i64(v: i64) -> Self {
        Decimal {
            mantissa: i128::from(v),
            scale: 0,
        }
    }

    /// The exact value of a `u64`.
    pub fn from_u64(v: u64) -> Self {
        Decimal {
            mantissa: i128::from(v),
            scale: 0,
        }
    }

    /// The decimal with the same shortest decimal representation as `v`; fails for NaN,
    /// infinities and values that need more digits than a decimal holds.
    pub fn from_f64(v: f64) -> Result<Self, DecimalError> {
        if !v.is_finite() {
            return Err(DecimalError::Overflow);
        }
        Decimal::parse(&format!("{v:e}"))
    }

    /// Parse `[+-]digits[.digits][(e|E)[+-]digits]`.
    pub fn parse(text: &str) -> Result<Self, DecimalError> {
        let b = text.as_bytes();
        let mut i = 0;
        let neg = match b.first() {
            Some(b'-') => {
                i += 1;
                true
            }
            Some(b'+') => {
                i += 1;
                false
            }
            _ => false,
        };
        let int_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let int_digits = &text[int_start..i];
        let mut frac_digits = "";
        if i < b.len() && b[i] == b'.' {
            i += 1;
            let fs = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            frac_digits = &text[fs..i];
        }
        if int_digits.is_empty() && frac_digits.is_empty() {
            return Err(DecimalError::Syntax);
        }
        let mut exp: i64 = 0;
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            let eneg = match b.get(i) {
                Some(b'-') => {
                    i += 1;
                    true
                }
                Some(b'+') => {
                    i += 1;
                    false
                }
                _ => false,
            };
            let es = i;
            while i < b.len() && b[i].is_ascii_digit() {
                if exp > 10_000 {
                    return Err(DecimalError::Overflow);
                }
                exp = exp * 10 + i64::from(b[i] - b'0');
                i += 1;
            }
            if es == i {
                return Err(DecimalError::Syntax);
            }
            if eneg {
                exp = -exp;
            }
        }
        if i != b.len() {
            return Err(DecimalError::Syntax);
        }
        let frac_digits = frac_digits.trim_end_matches('0');
        let digits = format!("{int_digits}{frac_digits}");
        let digits = digits.trim_start_matches('0');
        let mut scale = frac_digits.len() as i64 - exp;
        if digits.is_empty() {
            return Ok(Decimal::ZERO);
        }
        if digits.len() as u32 > MAX_DIGITS {
            return Err(DecimalError::Overflow);
        }
        let mut m: i128 = 0;
        for d in digits.bytes() {
            m = m * 10 + i128::from(d - b'0');
        }
        if scale < 0 {
            let up = u32::try_from(-scale).map_err(|_| DecimalError::Overflow)?;
            m = pow10(up)
                .and_then(|p| m.checked_mul(p))
                .ok_or(DecimalError::Overflow)?;
            scale = 0;
        }
        let scale = u32::try_from(scale).map_err(|_| DecimalError::Overflow)?;
        Decimal::new(if neg { -m } else { m }, scale)
    }

    /// Whether the value is zero.
    pub fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    /// Whether the value has no fractional part.
    pub fn is_integer(&self) -> bool {
        self.scale == 0
    }

    fn align(a: Decimal, b: Decimal) -> Result<(i128, i128, u32), DecimalError> {
        let s = a.scale.max(b.scale);
        let ma = pow10(s - a.scale)
            .and_then(|p| a.mantissa.checked_mul(p))
            .ok_or(DecimalError::Overflow)?;
        let mb = pow10(s - b.scale)
            .and_then(|p| b.mantissa.checked_mul(p))
            .ok_or(DecimalError::Overflow)?;
        Ok((ma, mb, s))
    }

    /// Exact sum.
    pub fn checked_add(self, o: Decimal) -> Result<Decimal, DecimalError> {
        let (a, b, s) = Decimal::align(self, o)?;
        Decimal::new(a.checked_add(b).ok_or(DecimalError::Overflow)?, s)
    }

    /// Exact difference.
    pub fn checked_sub(self, o: Decimal) -> Result<Decimal, DecimalError> {
        self.checked_add(o.neg())
    }

    /// Exact product.
    pub fn checked_mul(self, o: Decimal) -> Result<Decimal, DecimalError> {
        let m = self
            .mantissa
            .checked_mul(o.mantissa)
            .ok_or(DecimalError::Overflow)?;
        Decimal::new(m, self.scale + o.scale)
    }

    /// Quotient, exact when it fits in [`MAX_DIGITS`] digits and otherwise rounded half to
    /// even.
    pub fn checked_div(self, o: Decimal) -> Result<Decimal, DecimalError> {
        if o.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }
        let neg = (self.mantissa < 0) != (o.mantissa < 0);
        let n = self.mantissa.unsigned_abs();
        let d = o.mantissa.unsigned_abs();
        let mut q = n / d;
        let mut r = n % d;
        // value = q × 10^(e - k) where k is the number of fractional digits produced so far.
        let e = i64::from(o.scale) - i64::from(self.scale);
        let mut k: i64 = 0;
        while r != 0 && q * 10 + 9 < LIMIT && k - e < i64::from(MAX_SCALE) {
            r *= 10;
            q = q * 10 + r / d;
            r %= d;
            k += 1;
        }
        if r != 0 {
            let twice = r * 2;
            if twice > d || (twice == d && q % 2 == 1) {
                q += 1;
            }
        }
        if q >= LIMIT {
            return Err(DecimalError::Overflow);
        }
        let mut m = i128::try_from(q).map_err(|_| DecimalError::Overflow)?;
        let mut scale = k - e;
        if scale < 0 {
            let up = u32::try_from(-scale).map_err(|_| DecimalError::Overflow)?;
            m = pow10(up)
                .and_then(|p| m.checked_mul(p))
                .ok_or(DecimalError::Overflow)?;
            scale = 0;
        }
        let scale = u32::try_from(scale).map_err(|_| DecimalError::Overflow)?;
        Decimal::new(if neg { -m } else { m }, scale)
    }

    /// Exact remainder of truncated division; the sign follows the dividend.
    pub fn checked_rem(self, o: Decimal) -> Result<Decimal, DecimalError> {
        if o.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }
        let (a, b, s) = Decimal::align(self, o)?;
        Decimal::new(a % b, s)
    }

    /// Negation (always exact).
    #[allow(clippy::should_implement_trait)]
    pub fn neg(self) -> Decimal {
        Decimal {
            mantissa: -self.mantissa,
            scale: self.scale,
        }
    }

    /// The integer part, truncated toward zero, when it fits in an `i64`.
    pub fn trunc_i64(&self) -> Option<i64> {
        let p = pow10(self.scale)?;
        i64::try_from(self.mantissa / p).ok()
    }

    /// The integer part, truncated toward zero, when it fits in a `u64`.
    pub fn trunc_u64(&self) -> Option<u64> {
        let p = pow10(self.scale)?;
        u64::try_from(self.mantissa / p).ok()
    }

    /// The nearest `f64` (correctly rounded from the decimal text).
    pub fn to_f64(&self) -> f64 {
        let s: String = format!("{self}");
        s.parse::<f64>().unwrap_or(f64::NAN)
    }
}

impl Ord for Decimal {
    fn cmp(&self, o: &Self) -> Ordering {
        match Decimal::align(*self, *o) {
            Ok((a, b, _)) => a.cmp(&b),
            // Aligning overflowed, so the side scaled up has magnitude above 10^38 and the
            // other side's magnitude is below 10^34: the sign of the scaled side decides.
            Err(_) => {
                if self.scale < o.scale {
                    if self.mantissa < 0 {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                } else if o.mantissa < 0 {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
        }
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = format!("{}", self.mantissa.unsigned_abs());
        let sign = if self.mantissa < 0 { "-" } else { "" };
        let s = self.scale as usize;
        if s == 0 {
            return write!(f, "{sign}{digits}");
        }
        if digits.len() > s {
            let (i, fr) = digits.split_at(digits.len() - s);
            write!(f, "{sign}{i}.{fr}")
        } else {
            let zeros = "0".repeat(s - digits.len());
            write!(f, "{sign}0.{zeros}{digits}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn d(s: &str) -> Decimal {
        Decimal::parse(s).unwrap()
    }

    #[test]
    fn parse_and_format_are_canonical() {
        assert_eq!(d("1.50").to_string(), "1.5");
        assert_eq!(d("-0.0001").to_string(), "-0.0001");
        assert_eq!(d("12e3").to_string(), "12000");
        assert_eq!(d("1.25e-2").to_string(), "0.0125");
        assert_eq!(d("000").to_string(), "0");
        assert_eq!(d("1.50"), d("1.5"));
        assert!(Decimal::parse("1.2.3").is_err());
        assert!(Decimal::parse("").is_err());
        assert!(Decimal::parse("1".repeat(35).as_str()).is_err());
    }

    #[test]
    fn arithmetic_is_exact() {
        assert_eq!(d("0.1").checked_add(d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("1.1").checked_mul(d("1.1")).unwrap(), d("1.21"));
        assert_eq!(d("10").checked_div(d("4")).unwrap(), d("2.5"));
        assert_eq!(d("1").checked_div(d("3")).unwrap().to_string().len(), 36);
        assert_eq!(d("2").checked_div(d("3")).unwrap().to_string(), {
            let mut s = String::from("0.");
            s.push_str(&"6".repeat(33));
            s.push('7');
            s
        });
        assert_eq!(d("7.5").checked_rem(d("2")).unwrap(), d("1.5"));
        assert_eq!(
            d("1").checked_div(Decimal::ZERO),
            Err(DecimalError::DivisionByZero)
        );
        let big = d(&"9".repeat(34));
        assert_eq!(big.checked_add(d("1")), Err(DecimalError::Overflow));
    }

    #[test]
    fn ordering_is_numeric() {
        assert!(d("1.5") < d("1.51"));
        assert!(d("-2") < d("-1.99"));
        assert!(d(&"9".repeat(34)) > d("0.0000000000000000000000000000000001"));
        assert!(d("-1") < d("0.0000000000000000000000000000000001"));
    }
}
