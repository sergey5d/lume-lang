use std::borrow::Cow;

fn without_digit_separators(raw: &str) -> Cow<'_, str> {
    if raw.contains('_') {
        Cow::Owned(raw.replace('_', ""))
    } else {
        Cow::Borrowed(raw)
    }
}

pub(crate) fn parse_integer_literal(raw: &str, negative: bool) -> Option<i64> {
    let normalized = without_digit_separators(raw);
    let magnitude = normalized.parse::<u128>().ok()?;
    if negative {
        if magnitude == (1_u128 << 63) {
            return Some(i64::MIN);
        }
        let value = i64::try_from(magnitude).ok()?;
        return Some(-value);
    }
    i64::try_from(magnitude).ok()
}

pub(crate) fn parse_float_literal(raw: &str) -> Option<f64> {
    without_digit_separators(raw).parse::<f64>().ok()
}

pub(crate) fn checked_int_pow(value: i64, exponent: i64) -> Option<i64> {
    let mut remaining = u64::try_from(exponent).ok()?;
    let mut result = 1_i64;
    let mut factor = value;
    while remaining != 0 {
        if remaining & 1 != 0 {
            result = result.checked_mul(factor)?;
        }
        remaining >>= 1;
        if remaining != 0 {
            factor = factor.checked_mul(factor)?;
        }
    }
    Some(result)
}

pub(crate) fn render_float(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-Infinity".to_string()
        } else {
            "Infinity".to_string()
        };
    }

    let magnitude = value.abs();
    if magnitude != 0.0 && !(0.001..10_000_000.0).contains(&magnitude) {
        let rendered = format!("{value:E}");
        let (mantissa, _) = rendered.split_once('E').expect("scientific float");
        // Use the shortest digit count, with at least two significant digits.
        // Explicit precision gives the JVM's even rounding at decimal ties.
        let precision = mantissa
            .split_once('.')
            .map_or(1, |(_, fraction)| fraction.len());
        return format!("{value:.precision$E}");
    }

    let mut rendered = value.to_string();
    if !rendered.contains('.') {
        rendered.push_str(".0");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{checked_int_pow, parse_float_literal, parse_integer_literal, render_float};

    #[test]
    fn checked_integer_powers_preserve_signed_boundaries() {
        for (value, exponent, expected) in [
            (3, 39, Some(4_052_555_153_018_976_267)),
            (3, 40, None),
            (3, 41, None),
            (2, 62, Some(1_i64 << 62)),
            (2, 63, None),
            (-2, 63, Some(i64::MIN)),
            (i64::MIN, 1, Some(i64::MIN)),
            (i64::MIN, 2, None),
            (0, 0, Some(1)),
            (0, i64::MAX, Some(0)),
            (1, i64::MAX, Some(1)),
            (-1, i64::MAX, Some(-1)),
            (2, -1, None),
        ] {
            assert_eq!(
                checked_int_pow(value, exponent),
                expected,
                "{value}^{exponent}"
            );
        }
    }

    #[test]
    fn parses_complete_signed_integer_literal_boundary() {
        assert_eq!(
            parse_integer_literal("9223372036854775807", false),
            Some(i64::MAX)
        );
        assert_eq!(parse_integer_literal("9223372036854775808", false), None);
        assert_eq!(
            parse_integer_literal("9223372036854775808", true),
            Some(i64::MIN)
        );
        assert_eq!(parse_integer_literal("9223372036854775809", true), None);
    }

    #[test]
    fn parses_numeric_literals_with_digit_separators_and_exponents() {
        assert_eq!(parse_integer_literal("1_000_000", false), Some(1_000_000));
        assert_eq!(parse_float_literal("12.345_678"), Some(12.345_678));
        assert_eq!(parse_float_literal("1e-9"), Some(1e-9));
        assert_eq!(parse_float_literal("2.5e6"), Some(2.5e6));
    }

    #[test]
    fn renders_floats_at_notation_and_binary64_boundaries() {
        for (value, expected) in [
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (-1.25, "-1.25"),
            (0.001, "0.001"),
            (0.0001, "1.0E-4"),
            (-0.0001, "-1.0E-4"),
            (9_999_999.0, "9999999.0"),
            (10_000_000.0, "1.0E7"),
            (1.25e20, "1.25E20"),
            (1e-9, "1.0E-9"),
            (f64::MIN, "-1.7976931348623157E308"),
            (f64::MAX, "1.7976931348623157E308"),
            (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
            (f64::from_bits(1), "4.9E-324"),
            (f64::from_bits(2), "9.9E-324"),
            (
                f64::from_bits(0x4305_edd4_5e85_c45a),
                "7.715588606997872E14",
            ),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (f64::NAN, "NaN"),
        ] {
            let rendered = render_float(value);
            assert_eq!(rendered, expected);
            if !value.is_nan() {
                assert_eq!(rendered.parse::<f64>().unwrap().to_bits(), value.to_bits());
            }
        }
    }
}
