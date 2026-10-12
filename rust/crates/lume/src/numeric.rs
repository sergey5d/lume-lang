pub(crate) fn parse_integer_literal(raw: &str, negative: bool) -> Option<i64> {
    let magnitude = raw.parse::<u128>().ok()?;
    if negative {
        if magnitude == (1_u128 << 63) {
            return Some(i64::MIN);
        }
        let value = i64::try_from(magnitude).ok()?;
        return Some(-value);
    }
    i64::try_from(magnitude).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_integer_literal;

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
}
