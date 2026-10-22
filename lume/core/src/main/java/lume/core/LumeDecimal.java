package lume.core;

import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.regex.Pattern;

/** Immutable exact decimal with numeric (rather than scale-sensitive) equality. */
public final class LumeDecimal implements Hashed<LumeDecimal>, Comparable<LumeDecimal> {
    private static final long MAX_ALIGNMENT = 1_000_000;
    private static final Pattern DECIMAL = Pattern.compile(
            "[+-]?(?:[0-9]+(?:\\.[0-9]*)?|\\.[0-9]+)(?:[eE][+-]?[0-9]+)?");
    private final BigDecimal value;

    private LumeDecimal(BigDecimal value) { this.value = value; }

    public static LumeDecimal literal(String text) {
        try { return parseValue(text); }
        catch (IllegalArgumentException exception) { throw new LumePanic("invalid decimal literal"); }
    }

    public static Option<LumeDecimal> parse(String text) {
        try { return new Option.Some<>(parseValue(text)); }
        catch (IllegalArgumentException exception) { return Option.None.instance(); }
    }

    private static LumeDecimal parseValue(String text) {
        if (!DECIMAL.matcher(text).matches()) { throw new NumberFormatException(); }
        int e = Math.max(text.indexOf('e'), text.indexOf('E'));
        String mantissa = e < 0 ? text : text.substring(0, e);
        long exponent = e < 0 ? 0 : Long.parseLong(text.substring(e + 1));
        int point = mantissa.indexOf('.');
        long fraction = point < 0 ? 0 : mantissa.length() - point - 1;
        long scale;
        try { scale = Math.subtractExact(fraction, exponent); }
        catch (ArithmeticException exception) { throw new NumberFormatException(); }
        if (scale < Integer.MIN_VALUE || scale > Integer.MAX_VALUE) { throw new NumberFormatException(); }
        return new LumeDecimal(new BigDecimal(new BigInteger(mantissa.replace(".", "")), (int) scale));
    }

    public static LumeDecimal fromInt(long value) { return new LumeDecimal(BigDecimal.valueOf(value)); }

    public static Option<LumeDecimal> fromFloat(double value) {
        return Double.isFinite(value)
                ? new Option.Some<>(new LumeDecimal(BigDecimal.valueOf(value).stripTrailingZeros()))
                : Option.None.instance();
    }

    private static BigInteger power(long exponent) {
        if (exponent < 0 || exponent > MAX_ALIGNMENT) {
            throw new LumePanic("decimal arithmetic exceeds implementation resource limits");
        }
        return BigInteger.TEN.pow((int) exponent);
    }

    private static int checkedScale(long scale) {
        if (scale < Integer.MIN_VALUE || scale > Integer.MAX_VALUE) {
            throw new LumePanic("decimal scale out of range");
        }
        return (int) scale;
    }

    private void checkAlignment(LumeDecimal other) {
        if (Math.abs((long) value.scale() - other.value.scale()) > MAX_ALIGNMENT) {
            throw new LumePanic("decimal arithmetic exceeds implementation resource limits");
        }
    }

    public Long scale() { return (long) value.scale(); }
    public LumeDecimal neg() { return new LumeDecimal(value.negate()); }
    public LumeDecimal abs() { return new LumeDecimal(value.abs()); }
    public LumeDecimal add(LumeDecimal other) {
        checkAlignment(other);
        return new LumeDecimal(value.add(other.value));
    }
    public LumeDecimal sub(LumeDecimal other) { return add(other.neg()); }
    public LumeDecimal mul(LumeDecimal other) {
        int scale = checkedScale((long) value.scale() + other.value.scale());
        return new LumeDecimal(new BigDecimal(value.unscaledValue().multiply(other.value.unscaledValue()), scale));
    }

    public LumeDecimal div(LumeDecimal other) {
        if (other.value.signum() == 0) { throw new LumePanic("decimal division by zero"); }
        BigInteger common = value.unscaledValue().gcd(other.value.unscaledValue());
        BigInteger numerator = value.unscaledValue().divide(common);
        BigInteger denominator = other.value.unscaledValue().divide(common);
        if (denominator.signum() < 0) { numerator = numerator.negate(); denominator = denominator.negate(); }
        int twos = denominator.getLowestSetBit();
        denominator = denominator.shiftRight(twos);
        int fives = 0;
        while (denominator.mod(BigInteger.valueOf(5)).signum() == 0) {
            denominator = denominator.divide(BigInteger.valueOf(5));
            fives++;
        }
        if (!denominator.equals(BigInteger.ONE)) {
            throw new LumePanic("non-terminating decimal division; use divide with an explicit scale and rounding mode");
        }
        int digits = Math.max(twos, fives);
        if (digits > MAX_ALIGNMENT) { throw new LumePanic("decimal arithmetic exceeds implementation resource limits"); }
        int scale = checkedScale((long) value.scale() - other.value.scale() + digits);
        numerator = numerator.multiply(BigInteger.TWO.pow(digits - twos)).multiply(BigInteger.valueOf(5).pow(digits - fives));
        return new LumeDecimal(new BigDecimal(numerator, scale));
    }

    public LumeDecimal rem(LumeDecimal other) {
        if (other.value.signum() == 0) { throw new LumePanic("decimal remainder by zero"); }
        checkAlignment(other);
        int scale = Math.max(value.scale(), other.value.scale());
        BigInteger left = value.unscaledValue().multiply(power((long) scale - value.scale()));
        BigInteger right = other.value.unscaledValue().multiply(power((long) scale - other.value.scale()));
        return new LumeDecimal(new BigDecimal(left.remainder(right), scale));
    }

    public LumeDecimal divide(LumeDecimal other, long requestedScale, Rounding rounding) {
        if (other.value.signum() == 0) { throw new LumePanic("decimal division by zero"); }
        int scale = checkedScale(requestedScale);
        long exponent = (long) scale + other.value.scale() - value.scale();
        BigInteger numerator = value.unscaledValue();
        BigInteger denominator = other.value.unscaledValue();
        if (exponent >= 0) { numerator = numerator.multiply(power(exponent)); }
        else { denominator = denominator.multiply(power(-exponent)); }
        try {
            BigDecimal quotient = new BigDecimal(numerator).divide(new BigDecimal(denominator), 0, rounding.mode());
            return new LumeDecimal(new BigDecimal(quotient.unscaledValue(), scale));
        } catch (ArithmeticException exception) { throw new LumePanic("decimal rounding necessary"); }
    }

    public LumeDecimal round(long scale, Rounding rounding) { return divide(fromInt(1), scale, rounding); }
    public Option<Long> toInt() {
        try { return new Option.Some<>(value.longValueExact()); }
        catch (ArithmeticException exception) { return Option.None.instance(); }
    }
    public Double toFloat() { return value.doubleValue(); }
    @Override public Long compareTo(LumeDecimal other) { return (long) value.compareTo(other.value); }
    @Override public boolean equals(Object other) { return other instanceof LumeDecimal decimal && value.compareTo(decimal.value) == 0; }
    @Override public int hashCode() {
        if (value.signum() == 0) { return "0:0".hashCode(); }
        String digits = value.unscaledValue().toString();
        long scale = value.scale();
        int end = digits.length();
        while (digits.charAt(end - 1) == '0') { end--; scale--; }
        return (digits.substring(0, end) + ":" + scale).hashCode();
    }
    @Override public String toString() { return value.toString(); }
}
