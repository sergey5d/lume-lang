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

#[cfg(test)]
mod tests {
    use super::{parse_float_literal, parse_integer_literal};

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
}
