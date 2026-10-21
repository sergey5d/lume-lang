package lume.core;

import java.lang.reflect.InvocationTargetException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Locale;
import java.util.Objects;
import java.util.function.Supplier;
import java.util.regex.Pattern;
import java.util.stream.Collectors;

public final class LumeRuntime {
    private static String[] arguments = new String[0];

    private LumeRuntime() {
    }

    public static void setArgs(String[] args) {
        arguments = args == null ? new String[0] : args.clone();
    }

    public static LumeVector<String> args() {
        return LumeVector.from(Arrays.asList(arguments));
    }

    public static LumeMap<String, String> env() {
        var values = LumeMap.<String, String>empty();
        System.getenv().forEach(values::set);
        return values;
    }

    public static double sqrt(double value) {
        return Math.sqrt(value);
    }

    public static double log(double value) {
        return Math.log(value);
    }

    public static long addInt(long left, long right) {
        try {
            return Math.addExact(left, right);
        } catch (ArithmeticException error) {
            throw new LumePanic("integer overflow: " + left + " + " + right);
        }
    }

    public static long subInt(long left, long right) {
        try {
            return Math.subtractExact(left, right);
        } catch (ArithmeticException error) {
            throw new LumePanic("integer overflow: " + left + " - " + right);
        }
    }

    public static long mulInt(long left, long right) {
        try {
            return Math.multiplyExact(left, right);
        } catch (ArithmeticException error) {
            throw new LumePanic("integer overflow: " + left + " * " + right);
        }
    }

    public static long negInt(long value) {
        if (value == Long.MIN_VALUE) {
            throw new LumePanic("integer overflow: -(" + value + ")");
        }
        return -value;
    }

    public static long divInt(long left, long right) {
        if (right == 0) {
            throw new LumePanic("integer division by zero");
        }
        if (left == Long.MIN_VALUE && right == -1) {
            throw new LumePanic("integer overflow: " + left + " / " + right);
        }
        return left / right;
    }

    public static long remInt(long left, long right) {
        if (right == 0) {
            throw new LumePanic("integer remainder by zero");
        }
        return left % right;
    }

    public static long wrappingAdd(long value, long other) {
        return value + other;
    }

    public static long wrappingSub(long value, long other) {
        return value - other;
    }

    public static long wrappingMul(long value, long other) {
        return value * other;
    }

    public static long wrappingNeg(long value) {
        return -value;
    }

    public static Option<Long> checkedAdd(long value, long other) {
        try {
            return optionSome(Math.addExact(value, other));
        } catch (ArithmeticException error) {
            return optionNone();
        }
    }

    public static Option<Long> checkedSub(long value, long other) {
        try {
            return optionSome(Math.subtractExact(value, other));
        } catch (ArithmeticException error) {
            return optionNone();
        }
    }

    public static Option<Long> checkedMul(long value, long other) {
        try {
            return optionSome(Math.multiplyExact(value, other));
        } catch (ArithmeticException error) {
            return optionNone();
        }
    }

    public static long abs(long value) {
        if (value == Long.MIN_VALUE) {
            throw new LumePanic("integer overflow: (" + value + ").abs()");
        }
        return Math.abs(value);
    }

    public static double abs(double value) {
        return Math.abs(value);
    }

    public static long pow(long value, long exponent) {
        if (exponent < 0) {
            throw new LumePanic("Int.pow exponent must be non-negative");
        }
        try {
            return powExact(value, exponent);
        } catch (ArithmeticException error) {
            throw new LumePanic("integer overflow: " + value + ".pow(" + exponent + ")");
        }
    }

    public static Option<Long> checkedPow(long value, long exponent) {
        if (exponent < 0) {
            return optionNone();
        }
        try {
            return optionSome(powExact(value, exponent));
        } catch (ArithmeticException error) {
            return optionNone();
        }
    }

    private static long powExact(long value, long exponent) {
        long result = 1;
        long factor = value;
        long remaining = exponent;
        while (remaining != 0) {
            if ((remaining & 1) != 0) {
                result = Math.multiplyExact(result, factor);
            }
            remaining >>>= 1;
            if (remaining != 0) {
                factor = Math.multiplyExact(factor, factor);
            }
        }
        return result;
    }

    public static double pow(double value, long exponent) {
        return Math.pow(value, exponent);
    }

    public static double pow(double value, double exponent) {
        return Math.pow(value, exponent);
    }

    public static double floor(double value) {
        return Math.floor(value);
    }

    public static double ceil(double value) {
        return Math.ceil(value);
    }

    public static double round(double value) {
        if (!Double.isFinite(value) || value == 0.0) {
            return value;
        }
        return Math.copySign(Math.floor(Math.abs(value) + 0.5), value);
    }

    public static long toInt(double value) {
        return (long) value;
    }

    public static double toFloat(long value) {
        return (double) value;
    }

    public static LumeUnit print(Object... values) {
        System.out.print(join(values));
        return LumeUnit.INSTANCE;
    }

    public static LumeUnit println(Object... values) {
        System.out.println(join(values));
        return LumeUnit.INSTANCE;
    }

    public static LumeUnit printf(String format, Object... values) {
        System.out.printf(format, values);
        return LumeUnit.INSTANCE;
    }

    public static LumeUnit assertTrue(Boolean condition, String message) {
        if (!Boolean.TRUE.equals(condition)) {
            throw new LumePanic(message);
        }
        return LumeUnit.INSTANCE;
    }

    public static <E> Result<LumeUnit, E> ensure(Boolean condition, Supplier<E> error) {
        if (Boolean.TRUE.equals(condition)) {
            return new Result.Ok<>(LumeUnit.INSTANCE);
        }
        return new Result.Err<>(error.get());
    }

    public static <T> Option<T> optionSome(T value) {
        return new Option.Some<>(value);
    }

    public static <T> Option<T> optionNone() {
        return Option.None.instance();
    }

    public static Option<Long> parseInt(String text) {
        try {
            return optionSome(Long.parseLong(text));
        } catch (NumberFormatException err) {
            return optionNone();
        }
    }

    public static Option<Double> parseFloat(String text) {
        try {
            return optionSome(Double.parseDouble(text));
        } catch (NumberFormatException err) {
            return optionNone();
        }
    }

    public static LumeVector<String> stringSplit(String text, String separator) {
        var parts = new ArrayList<String>();
        if (separator.isEmpty()) {
            parts.add("");
            text.codePoints().forEach(codePoint -> parts.add(new String(Character.toChars(codePoint))));
            parts.add("");
            return LumeVector.from(parts);
        }

        int start = 0;
        int next;
        while ((next = text.indexOf(separator, start)) >= 0) {
            parts.add(text.substring(start, next));
            start = next + separator.length();
        }
        parts.add(text.substring(start));
        return LumeVector.from(parts);
    }

    public static LumeVector<String> stringSplitRegex(String text, String pattern) {
        return LumeVector.from(Arrays.asList(text.split(pattern, -1)));
    }

    public static String stringTrim(String text) {
        return text.strip();
    }

    public static String stringTrimLeft(String text) {
        return text.stripLeading();
    }

    public static String stringTrimRight(String text) {
        return text.stripTrailing();
    }

    public static String stringToLower(String text) {
        return text.toLowerCase(Locale.ROOT);
    }

    public static String stringToUpper(String text) {
        return text.toUpperCase(Locale.ROOT);
    }

    public static Boolean stringContains(String text, String part) {
        return text.contains(part);
    }

    public static Boolean stringStartsWith(String text, String prefix) {
        return text.startsWith(prefix);
    }

    public static Boolean stringEndsWith(String text, String suffix) {
        return text.endsWith(suffix);
    }

    public static Option<Long> stringIndexOf(String text, String part) {
        int utf16Index = text.indexOf(part);
        return utf16Index < 0
            ? optionNone()
            : optionSome((long) text.codePointCount(0, utf16Index));
    }

    public static String stringReplaceFirst(String text, String target, String replacement) {
        int index = text.indexOf(target);
        if (index < 0) {
            return text;
        }
        return text.substring(0, index) + replacement + text.substring(index + target.length());
    }

    public static String stringReplaceAll(String text, String target, String replacement) {
        return text.replace(target, replacement);
    }

    public static String stringReplaceFirstRegex(String text, String pattern, String replacement) {
        return Pattern.compile(pattern).matcher(text).replaceFirst(replacement);
    }

    public static String stringReplaceAllRegex(String text, String pattern, String replacement) {
        return Pattern.compile(pattern).matcher(text).replaceAll(replacement);
    }

    public static Long stringSize(String text) {
        return (long) text.codePointCount(0, text.length());
    }

    public static Option<Integer> stringRuneAt(String text, Long index) {
        if (index < 0 || index > Integer.MAX_VALUE) {
            return optionNone();
        }
        int scalarIndex = index.intValue();
        int size = text.codePointCount(0, text.length());
        if (scalarIndex >= size) {
            return optionNone();
        }
        int offset = text.offsetByCodePoints(0, scalarIndex);
        return optionSome(text.codePointAt(offset));
    }

    public static String stringSlice(String text) {
        return stringSlice(text, 0, stringSize(text));
    }

    public static String stringSlice(String text, long start) {
        return stringSlice(text, start, stringSize(text));
    }

    public static String stringSlice(String text, long start, long end) {
        long size = stringSize(text);
        if (start < 0 || end < start || end > size) {
            throw new LumePanic(
                "Str.slice range " + start + ":" + end + " is out of bounds for size " + size
            );
        }
        int startOffset = text.offsetByCodePoints(0, Math.toIntExact(start));
        int endOffset = text.offsetByCodePoints(0, Math.toIntExact(end));
        return text.substring(startOffset, endOffset);
    }

    public static Boolean extractSuccessIsSet(Object value) {
        if (value instanceof Option<?> option) {
            return option.isDefined();
        }
        if (value instanceof Result<?, ?> result) {
            return result.isOk();
        }
        if (value instanceof Either<?, ?> either) {
            return either.isRight();
        }
        return false;
    }

    public static Boolean variantIs(Object value, String caseName) {
        return value != null && value.getClass().getSimpleName().equals(caseName);
    }

    public static Object variantField(Object value, String fieldName) {
        if (value == null) {
            throw new LumePanic("cannot read enum field '" + fieldName + "' from null");
        }

        try {
            var method = value.getClass().getMethod(fieldName);
            return method.invoke(value);
        } catch (NoSuchMethodException err) {
            // Pattern matching may probe fields before the case guard proves that this case matched.
            return null;
        } catch (IllegalAccessException | InvocationTargetException err) {
            throw new LumePanic("failed to read enum field '" + fieldName + "': " + err.getMessage());
        }
    }

    public static Object patternField(Object value, String fieldName) {
        if (value == null) {
            return null;
        }

        try {
            var method = value.getClass().getMethod(fieldName);
            return method.invoke(value);
        } catch (NoSuchMethodException err) {
            try {
                var field = value.getClass().getDeclaredField(fieldName);
                field.setAccessible(true);
                return field.get(value);
            } catch (NoSuchFieldException missingField) {
                // A failed outer type/case test makes this field irrelevant to the pattern.
                return null;
            } catch (IllegalAccessException fieldAccess) {
                throw new LumePanic(
                        "failed to read pattern field '" + fieldName + "': " + fieldAccess.getMessage());
            }
        } catch (IllegalAccessException | InvocationTargetException err) {
            throw new LumePanic("failed to read pattern field '" + fieldName + "': " + err.getMessage());
        }
    }

    public static LumeType runtimeTypeOf(Object value) {
        if (value == null) {
            return LumeType.primitive("Null");
        }
        if (value instanceof String) {
            return LumeType.primitive("Str");
        }
        if (value instanceof Boolean) {
            return LumeType.primitive("Bool");
        }
        if (value instanceof Long || value instanceof Integer || value instanceof Short || value instanceof Byte) {
            return LumeType.primitive("Int");
        }
        if (value instanceof Double || value instanceof Float) {
            return LumeType.primitive("Float");
        }
        if (value instanceof LumeUnit) {
            return LumeType.primitive("Unit");
        }

        if (value instanceof LumeTyped typed) {
            return typed.runtimeType();
        }

        return LumeType.classType(
                value.getClass().getSimpleName(),
                value.getClass().getName(),
                new LumeField[] {},
                new LumeMethod[] {});
    }

    public static Long hashValue(Object value) {
        if (value instanceof Hashed<?> hashed) {
            return hashed.hash();
        }
        return (long) Objects.hashCode(value);
    }

    public static ReferenceId referenceIdOf(Object value) {
        if (value == null) {
            throw new LumePanic("referenceId is not available on null");
        }
        return new ReferenceId(value);
    }

    public static Object probeSuccessValue(Object value) {
        if (value instanceof Option.Some<?> some) {
            return some.value();
        }
        if (value instanceof Result.Ok<?, ?> ok) {
            return ok.value();
        }
        if (value instanceof Either.Right<?, ?> right) {
            return right.value();
        }
        return LumeUnit.INSTANCE;
    }

    public static Object extractSuccessValue(Object value) {
        if (value instanceof Option.Some<?> some) {
            return some.value();
        }
        if (value instanceof Option.None<?>) {
            throw new LumePanic("expected Option.Some");
        }
        if (value instanceof Result.Ok<?, ?> ok) {
            return ok.value();
        }
        if (value instanceof Result.Err<?, ?>) {
            throw new LumePanic("expected Result.Ok");
        }
        if (value instanceof Either.Right<?, ?> right) {
            return right.value();
        }
        if (value instanceof Either.Left<?, ?>) {
            throw new LumePanic("expected Either.Right");
        }
        throw new LumePanic("expected success value");
    }

    public static Object indexValue(Object base, Object index) {
        long offset = ((Number) index).longValue();
        Option<?> value;
        if (base instanceof LumeVector<?> list) {
            value = list.at(offset);
        } else if (base instanceof LumeArray<?> array) {
            value = array.at(offset);
        } else {
            throw new LumePanic("value is not indexable");
        }

        if (value instanceof Option.Some<?> some) {
            return some.value();
        }
        throw new LumePanic("index out of bounds: " + offset);
    }

    public static long listLen(Object value) {
        if (value instanceof LumeVector<?> list) {
            return list.size();
        }
        if (value instanceof LumeArray<?> array) {
            return array.size();
        }
        throw new LumePanic("expected Vector or Array");
    }

    public static Object listGet(Object value, Object index) {
        var offset = ((Number) index).longValue();
        Option<?> item;
        if (value instanceof LumeVector<?> list) {
            item = list.at(offset);
        } else if (value instanceof LumeArray<?> array) {
            item = array.at(offset);
        } else {
            throw new LumePanic("expected Vector or Array");
        }
        if (item instanceof Option.Some<?> some) {
            return some.value();
        }
        throw new LumePanic("sequence pattern index out of bounds");
    }

    public static LumeVector<?> listSlice(Object value, Object start) {
        java.util.List<?> values;
        if (value instanceof LumeVector<?> list) {
            values = list.asJava();
        } else if (value instanceof LumeArray<?> array) {
            values = array.asJava();
        } else {
            throw new LumePanic("expected Vector or Array");
        }
        var offset = Math.max(0, ((Number) start).intValue());
        if (offset >= values.size()) {
            return LumeVector.empty();
        }
        return LumeVector.from(values.subList(offset, values.size()));
    }

    public static <T> LumeIterator<T> iterInit(Object source) {
        return LumeIterator.from(source);
    }

    public static Boolean iterHasNext(Object iterator) {
        return ((LumeIterator<?>) iterator).hasNext();
    }

    @SuppressWarnings("unchecked")
    public static <T> T iterNext(Object iterator) {
        return ((LumeIterator<T>) iterator).next();
    }

    private static String join(Object... values) {
        return Arrays.stream(values)
                .map(String::valueOf)
                .collect(Collectors.joining(" "));
    }
}
