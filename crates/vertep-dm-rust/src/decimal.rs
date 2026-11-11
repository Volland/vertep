//! Exact decimals for the `decimal` primitive of the type language.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{Add, Mul, Neg, Sub};
use core::str::FromStr;

/// An exact decimal number: `mantissa / 10^scale`.
///
/// The canonical text is `-?[0-9]+(\.[0-9]+)?` without trailing fraction zeros, the encoding
/// the type language gives `decimal` values. Up to 38 significant digits are exact; arithmetic
/// that would need more panics like integer overflow does, and the `checked_*` methods return
/// `None` instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decimal {
    mantissa: i128,
    scale: u32,
}

/// Why a string is not a decimal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseDecimalError {
    /// The rejected text.
    pub text: String,
}

impl fmt::Display for ParseDecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` is not a decimal of at most 38 digits (-?[0-9]+(.[0-9]+)?)",
            self.text
        )
    }
}

impl std::error::Error for ParseDecimalError {}

const MAX_SCALE: u32 = 38;

fn pow10(n: u32) -> Option<i128> {
    10i128.checked_pow(n)
}

impl Decimal {
    /// Zero.
    pub const ZERO: Decimal = Decimal {
        mantissa: 0,
        scale: 0,
    };
    /// One.
    pub const ONE: Decimal = Decimal {
        mantissa: 1,
        scale: 0,
    };

    /// `mantissa / 10^scale`. `scale` is capped at 38.
    pub fn new(mantissa: i64, scale: u32) -> Decimal {
        Decimal {
            mantissa: i128::from(mantissa),
            scale: scale.min(MAX_SCALE),
        }
        .normalized()
    }

    /// The decimal with trailing fraction zeros removed.
    fn normalized(self) -> Decimal {
        let mut d = self;
        if d.mantissa == 0 {
            return Decimal::ZERO;
        }
        while d.scale > 0 && d.mantissa % 10 == 0 {
            d.mantissa /= 10;
            d.scale -= 1;
        }
        d
    }

    /// Both values at the same scale, or `None` on overflow.
    fn aligned(self, other: Decimal) -> Option<(i128, i128, u32)> {
        let scale = self.scale.max(other.scale);
        let a = self.mantissa.checked_mul(pow10(scale - self.scale)?)?;
        let b = other.mantissa.checked_mul(pow10(scale - other.scale)?)?;
        Some((a, b, scale))
    }

    /// `self + other`, or `None` on overflow.
    pub fn checked_add(self, other: Decimal) -> Option<Decimal> {
        let (a, b, scale) = self.aligned(other)?;
        Some(
            Decimal {
                mantissa: a.checked_add(b)?,
                scale,
            }
            .normalized(),
        )
    }

    /// `self - other`, or `None` on overflow.
    pub fn checked_sub(self, other: Decimal) -> Option<Decimal> {
        self.checked_add(Decimal {
            mantissa: other.mantissa.checked_neg()?,
            scale: other.scale,
        })
    }

    /// `self * other`, or `None` on overflow.
    pub fn checked_mul(self, other: Decimal) -> Option<Decimal> {
        let scale = self.scale + other.scale;
        if scale > MAX_SCALE {
            return None;
        }
        Some(
            Decimal {
                mantissa: self.mantissa.checked_mul(other.mantissa)?,
                scale,
            }
            .normalized(),
        )
    }

    /// Whether the value is below zero.
    pub fn is_negative(&self) -> bool {
        self.mantissa < 0
    }

    /// The value as an `f64`, rounded.
    pub fn to_f64(&self) -> f64 {
        // The canonical text parses to the nearest f64.
        self.to_string().parse().unwrap_or(f64::NAN)
    }

    /// The nearest decimal to a finite `f64` (its shortest round-trip text), or `None` for
    /// infinities and NaN.
    pub fn from_f64(f: f64) -> Option<Decimal> {
        if !f.is_finite() {
            return None;
        }
        let text = format!("{f}");
        if text.contains('e') {
            return None;
        }
        text.parse().ok()
    }

    fn magnitude_parts(&self) -> (String, String) {
        let digits = self.mantissa.unsigned_abs().to_string();
        let scale = self.scale as usize;
        if digits.len() > scale {
            let (i, f) = digits.split_at(digits.len() - scale);
            (i.to_string(), f.to_string())
        } else {
            ("0".to_string(), format!("{digits:0>scale$}"))
        }
    }

    fn cmp_magnitude(&self, other: &Decimal) -> Ordering {
        let (ai, af) = self.magnitude_parts();
        let (bi, bf) = other.magnitude_parts();
        ai.len()
            .cmp(&bi.len())
            .then_with(|| ai.cmp(&bi))
            .then_with(|| {
                let n = af.len().max(bf.len());
                format!("{af:0<n$}").cmp(&format!("{bf:0<n$}"))
            })
    }
}

impl FromStr for Decimal {
    type Err = ParseDecimalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseDecimalError {
            text: s.to_string(),
        };
        if !vertep_core::types::prim::is_decimal(s) {
            return Err(err());
        }
        let (neg, body) = match s.strip_prefix('-') {
            Some(b) => (true, b),
            None => (false, s),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        let frac = frac.trim_end_matches('0');
        let int = int.trim_start_matches('0');
        if frac.len() > MAX_SCALE as usize {
            return Err(err());
        }
        let mut mantissa: i128 = 0;
        for b in int.bytes().chain(frac.bytes()) {
            mantissa = mantissa
                .checked_mul(10)
                .and_then(|m| m.checked_add(i128::from(b - b'0')))
                .ok_or_else(err)?;
        }
        Ok(Decimal {
            mantissa: if neg { -mantissa } else { mantissa },
            scale: frac.len() as u32,
        }
        .normalized())
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = self.normalized();
        let (i, frac) = d.magnitude_parts();
        if d.mantissa < 0 {
            f.write_str("-")?;
        }
        f.write_str(&i)?;
        if !frac.is_empty() {
            write!(f, ".{frac}")?;
        }
        Ok(())
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Decimal {}

impl Hash for Decimal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let n = self.normalized();
        n.mantissa.hash(state);
        n.scale.hash(state);
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.mantissa.signum(), other.mantissa.signum()) {
            (a, b) if a != b => a.cmp(&b),
            (0, _) => Ordering::Equal,
            (1, _) => self.cmp_magnitude(other),
            _ => other.cmp_magnitude(self),
        }
    }
}

impl From<i64> for Decimal {
    fn from(v: i64) -> Self {
        Decimal::new(v, 0)
    }
}

impl From<i32> for Decimal {
    fn from(v: i32) -> Self {
        Decimal::new(i64::from(v), 0)
    }
}

impl Add for Decimal {
    type Output = Decimal;
    fn add(self, rhs: Decimal) -> Decimal {
        self.checked_add(rhs).expect("decimal addition overflowed")
    }
}

impl Sub for Decimal {
    type Output = Decimal;
    fn sub(self, rhs: Decimal) -> Decimal {
        self.checked_sub(rhs)
            .expect("decimal subtraction overflowed")
    }
}

impl Mul for Decimal {
    type Output = Decimal;
    fn mul(self, rhs: Decimal) -> Decimal {
        self.checked_mul(rhs)
            .expect("decimal multiplication overflowed")
    }
}

impl Neg for Decimal {
    type Output = Decimal;
    fn neg(self) -> Decimal {
        Decimal {
            mantissa: -self.mantissa,
            scale: self.scale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn parse_print_and_compare() {
        assert_eq!(d("1.50").to_string(), "1.5");
        assert_eq!(d("-0.0").to_string(), "0");
        assert_eq!(d("007").to_string(), "7");
        assert_eq!(d("0.001").to_string(), "0.001");
        assert!(d("10") > d("9.99"));
        assert!(d("-10") < d("-9.99"));
        assert!(d("0.5") > Decimal::ZERO);
        assert_eq!(d("1.0"), Decimal::ONE);
        assert!("1e3".parse::<Decimal>().is_err());
        assert!("".parse::<Decimal>().is_err());
    }

    #[test]
    fn arithmetic_is_exact() {
        assert_eq!(d("0.1") + d("0.2"), d("0.3"));
        assert_eq!(d("1.5") * d("2"), d("3"));
        assert_eq!(d("1") - d("1.25"), d("-0.25"));
        assert_eq!(Decimal::from_f64(0.1), Some(d("0.1")));
        assert_eq!(d("2.5").to_f64(), 2.5);
    }
}
