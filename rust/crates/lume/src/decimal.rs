use std::{cmp::Ordering, fmt};

use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{Signed, ToPrimitive, Zero};

// Bound temporary powers of ten, not numeric precision or stored scale.
pub(crate) const MAX_ALIGNMENT: i64 = 1_000_000;

pub(crate) fn is_rounding_name(name: &str) -> bool {
    matches!(
        name,
        "Up" | "Down" | "Ceiling" | "Floor" | "HalfUp" | "HalfDown" | "HalfEven" | "Unnecessary"
    )
}

#[derive(Clone, Debug)]
pub(crate) struct Decimal {
    coefficient: BigInt,
    pub(crate) scale: i32,
}

impl Decimal {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (mantissa, exponent) = match text.find(['e', 'E']) {
            Some(index) => (&text[..index], text[index + 1..].parse::<i64>().ok()?),
            None => (text, 0),
        };
        let unsigned = mantissa.strip_prefix(['+', '-']).unwrap_or(mantissa);
        let mut point = false;
        let mut fraction = 0_i64;
        let mut digits = String::new();
        for ch in unsigned.chars() {
            match ch {
                '.' if !point => point = true,
                '0'..='9' => {
                    digits.push(ch);
                    fraction += i64::from(point);
                }
                _ => return None,
            }
        }
        if digits.is_empty() {
            return None;
        }
        let scale = i32::try_from(fraction.checked_sub(exponent)?).ok()?;
        let mut coefficient = BigInt::parse_bytes(digits.as_bytes(), 10)?;
        if mantissa.starts_with('-') {
            coefficient = -coefficient;
        }
        Some(Self { coefficient, scale })
    }

    pub(crate) fn literal(raw: &str) -> Option<Self> {
        Self::parse(&raw.strip_suffix('d')?.replace('_', ""))
    }

    pub(crate) fn from_int(value: i64) -> Self {
        Self {
            coefficient: value.into(),
            scale: 0,
        }
    }

    pub(crate) fn from_float(value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let parsed = Self::parse(&crate::numeric::render_float(value))?;
        let key = parsed.canonical_key();
        let (coefficient, scale) = key.split_once(':')?;
        Some(Self {
            coefficient: coefficient.parse().ok()?,
            scale: scale.parse().ok()?,
        })
    }

    pub(crate) fn neg(&self) -> Self {
        Self {
            coefficient: -&self.coefficient,
            scale: self.scale,
        }
    }

    pub(crate) fn abs(&self) -> Self {
        Self {
            coefficient: self.coefficient.abs(),
            scale: self.scale,
        }
    }

    fn power(exponent: i64) -> Result<BigInt, &'static str> {
        if !(0..=MAX_ALIGNMENT).contains(&exponent) {
            return Err("decimal arithmetic exceeds implementation resource limits");
        }
        Ok(BigInt::from(10).pow(exponent as u32))
    }

    fn aligned(&self, other: &Self) -> Result<(BigInt, BigInt, i32), &'static str> {
        let scale = self.scale.max(other.scale);
        Ok((
            &self.coefficient * Self::power(i64::from(scale) - i64::from(self.scale))?,
            &other.coefficient * Self::power(i64::from(scale) - i64::from(other.scale))?,
            scale,
        ))
    }

    pub(crate) fn add(&self, other: &Self) -> Result<Self, &'static str> {
        let (left, right, scale) = self.aligned(other)?;
        Ok(Self {
            coefficient: left + right,
            scale,
        })
    }

    pub(crate) fn sub(&self, other: &Self) -> Result<Self, &'static str> {
        self.add(&other.neg())
    }

    pub(crate) fn mul(&self, other: &Self) -> Result<Self, &'static str> {
        let scale = self
            .scale
            .checked_add(other.scale)
            .ok_or("decimal scale out of range")?;
        Ok(Self {
            coefficient: &self.coefficient * &other.coefficient,
            scale,
        })
    }

    pub(crate) fn div(&self, other: &Self) -> Result<Self, &'static str> {
        if other.coefficient.is_zero() {
            return Err("decimal division by zero");
        }
        let preferred = i64::from(self.scale) - i64::from(other.scale);
        let common = self.coefficient.gcd(&other.coefficient);
        let mut numerator = &self.coefficient / &common;
        let mut denominator = &other.coefficient / common;
        if denominator.is_negative() {
            denominator = -denominator;
            numerator = -numerator;
        }
        let mut twos = 0_i64;
        let mut fives = 0_i64;
        while denominator.is_even() {
            denominator /= 2;
            twos += 1;
        }
        while (&denominator % 5_u8).is_zero() {
            denominator /= 5;
            fives += 1;
        }
        if denominator != BigInt::from(1) {
            return Err(
                "non-terminating decimal division; use divide with an explicit scale and rounding mode",
            );
        }
        let digits = twos.max(fives);
        if digits > MAX_ALIGNMENT {
            return Err("decimal arithmetic exceeds implementation resource limits");
        }
        let scale = i32::try_from(preferred + digits).map_err(|_| "decimal scale out of range")?;
        numerator *= BigInt::from(2).pow((digits - twos) as u32);
        numerator *= BigInt::from(5).pow((digits - fives) as u32);
        Ok(Self {
            coefficient: numerator,
            scale,
        })
    }

    pub(crate) fn rem(&self, other: &Self) -> Result<Self, &'static str> {
        if other.coefficient.is_zero() {
            return Err("decimal remainder by zero");
        }
        let (left, right, scale) = self.aligned(other)?;
        Ok(Self {
            coefficient: left % right,
            scale,
        })
    }

    pub(crate) fn divide(
        &self,
        other: &Self,
        scale: i64,
        rounding: &str,
    ) -> Result<Self, &'static str> {
        if other.coefficient.is_zero() {
            return Err("decimal division by zero");
        }
        let scale = i32::try_from(scale).map_err(|_| "decimal scale out of range")?;
        let exponent = i64::from(scale) + i64::from(other.scale) - i64::from(self.scale);
        let (numerator, denominator) = if exponent >= 0 {
            (
                &self.coefficient * Self::power(exponent)?,
                other.coefficient.clone(),
            )
        } else {
            (
                self.coefficient.clone(),
                &other.coefficient * Self::power(-exponent)?,
            )
        };
        let (mut quotient, remainder) = numerator.div_rem(&denominator);
        if !remainder.is_zero() {
            let positive = numerator.sign() == denominator.sign();
            let halfway = (remainder.abs() * 2_u8).cmp(&denominator.abs());
            let increment = match rounding {
                "Up" => true,
                "Down" => false,
                "Ceiling" => positive,
                "Floor" => !positive,
                "HalfUp" => halfway != Ordering::Less,
                "HalfDown" => halfway == Ordering::Greater,
                "HalfEven" => {
                    halfway == Ordering::Greater
                        || (halfway == Ordering::Equal && quotient.is_odd())
                }
                "Unnecessary" => return Err("decimal rounding necessary"),
                _ => return Err("invalid decimal rounding mode"),
            };
            if increment {
                quotient += if positive { 1 } else { -1 };
            }
        }
        Ok(Self {
            coefficient: quotient,
            scale,
        })
    }

    pub(crate) fn to_int(&self) -> Option<i64> {
        if self.coefficient.is_zero() {
            return Some(0);
        }
        let digits = self.coefficient.abs().to_str_radix(10).len() as i64;
        if self.scale > 0 {
            if i64::from(self.scale) >= digits {
                return None;
            }
            let (whole, remainder) = self
                .coefficient
                .div_rem(&Self::power(i64::from(self.scale)).ok()?);
            if !remainder.is_zero() {
                return None;
            }
            whole.to_i64()
        } else {
            if digits - i64::from(self.scale) > 19 {
                return None;
            }
            (&self.coefficient * Self::power(-i64::from(self.scale)).ok()?).to_i64()
        }
    }

    pub(crate) fn to_float(&self) -> f64 {
        self.to_string()
            .parse()
            .expect("a finite decimal spelling is valid Float input")
    }

    pub(crate) fn canonical_key(&self) -> String {
        if self.coefficient.is_zero() {
            return "0:0".to_string();
        }
        let mut digits = self.coefficient.to_str_radix(10);
        let mut scale = i64::from(self.scale);
        while digits.ends_with('0') {
            digits.pop();
            scale -= 1;
        }
        format!("{digits}:{scale}")
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Decimal {}
impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        let sign = self.coefficient.sign().cmp(&other.coefficient.sign());
        if sign != Ordering::Equal {
            return sign;
        }
        if self.coefficient.is_zero() {
            return Ordering::Equal;
        }
        let left = self.coefficient.abs().to_str_radix(10);
        let right = other.coefficient.abs().to_str_radix(10);
        let exponent = (left.len() as i64 - i64::from(self.scale))
            .cmp(&(right.len() as i64 - i64::from(other.scale)));
        let order = exponent.then_with(|| {
            let width = left.len().max(right.len());
            left.bytes()
                .chain(std::iter::repeat(b'0'))
                .take(width)
                .cmp(right.bytes().chain(std::iter::repeat(b'0')).take(width))
        });
        if self.coefficient.is_negative() {
            order.reverse()
        } else {
            order
        }
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = self.coefficient.abs().to_str_radix(10);
        let adjusted = digits.len() as i64 - 1 - i64::from(self.scale);
        if self.coefficient.is_negative() {
            f.write_str("-")?;
        }
        if self.scale >= 0 && adjusted >= -6 {
            let point = digits.len() as i64 - i64::from(self.scale);
            if self.scale == 0 {
                f.write_str(&digits)
            } else if point > 0 {
                write!(
                    f,
                    "{}.{}",
                    &digits[..point as usize],
                    &digits[point as usize..]
                )
            } else {
                write!(f, "0.{}{}", "0".repeat((-point) as usize), digits)
            }
        } else {
            f.write_str(&digits[..1])?;
            if digits.len() > 1 {
                write!(f, ".{}", &digits[1..])?;
            }
            write!(f, "E{adjusted:+}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Decimal;

    fn d(value: &str) -> Decimal {
        Decimal::parse(value).unwrap()
    }

    #[test]
    fn exact_decimal_arithmetic_and_numeric_equality() {
        assert_eq!(d("0.1").add(&d("0.2")).unwrap(), d("0.3"));
        assert_eq!(d("19.99").mul(&d("3")).unwrap().to_string(), "59.97");
        assert_eq!(d("1.00").div(&d("2")).unwrap().to_string(), "0.50");
        assert_eq!(d("100").div(&d("2.0")).unwrap().to_string(), "5E+1");
        assert!(d("1").div(&d("3")).is_err());
        assert_eq!(d("1.0").canonical_key(), d("1.00").canonical_key());
        assert!(d("-2") < d("-1"));
        assert!(d("0") < d("1"));
        assert_eq!(d("9223372036854775807.0").to_int(), Some(i64::MAX));
        assert_eq!(d("9223372036854775808").to_int(), None);
        assert_eq!(d("1.1").to_int(), None);
        assert_eq!(d("1e2147483647").to_string(), "1E+2147483647");
        assert!(d("1e2147483647") > d("1e2147483646"));
    }

    #[test]
    fn explicit_rounding_handles_signs_and_ties() {
        for (value, mode, expected) in [
            ("1.25", "HalfEven", "1.2"),
            ("1.35", "HalfEven", "1.4"),
            ("-1.25", "HalfUp", "-1.3"),
            ("-1.25", "HalfDown", "-1.2"),
            ("-1.21", "Floor", "-1.3"),
            ("-1.21", "Ceiling", "-1.2"),
            ("1.21", "Up", "1.3"),
            ("1.29", "Down", "1.2"),
        ] {
            assert_eq!(
                d(value).divide(&d("1"), 1, mode).unwrap().to_string(),
                expected
            );
        }
        assert!(d("1.21").divide(&d("1"), 1, "Unnecessary").is_err());
    }

    #[test]
    fn decimal_boundaries_remain_exact_and_bounded() {
        for text in [
            "",
            ".",
            "1_0",
            "NaN",
            "Infinity",
            " 1",
            "1 ",
            "1e",
            "1e2147483649",
        ] {
            assert!(Decimal::parse(text).is_none(), "{text}");
        }
        assert_eq!(d("-9223372036854775808.00").to_int(), Some(i64::MIN));
        assert_eq!(d("-9223372036854775809").to_int(), None);
        assert_eq!(d("0e-2147483647").to_int(), Some(0));
        assert_eq!(d("1e2147483647").to_int(), None);
        assert_eq!(d("1e2147483647").to_float(), f64::INFINITY);
        assert_eq!(d("1e-2147483647").to_float(), 0.0);
        assert!(d("1").add(&d("1e-1000001")).is_err());
        assert!(d("1e2147483647").mul(&d("1e2")).is_err());
        assert_eq!(
            d("123")
                .divide(&d("1"), -1, "HalfEven")
                .unwrap()
                .to_string(),
            "1.2E+2"
        );
        assert_eq!(d("1").divide(&d("-8"), 2, "HalfEven").unwrap(), d("-0.12"));
        assert_eq!(d("0e2147483647").canonical_key(), "0:0");
        assert_eq!(Decimal::from_float(100.0).unwrap().scale, -2);
        assert!(Decimal::from_float(f64::NAN).is_none());
        assert_eq!(
            Decimal::from_float(f64::from_bits(1)).unwrap(),
            d("4.9e-324")
        );
    }
}
