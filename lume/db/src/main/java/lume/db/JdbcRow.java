package lume.db;

import java.util.LinkedHashMap;
import java.util.Locale;

import lume.core.LumeList;
import lume.core.LumeRuntime;
import lume.core.LumeField;
import lume.core.LumeType;
import lume.core.Option;
import lume.core.Result;

public final class JdbcRow {
    private final LinkedHashMap<String, Object> values;

    JdbcRow(LinkedHashMap<String, Object> values) {
        this.values = new LinkedHashMap<>(values);
    }

    public LumeList<String> columns() {
        return LumeList.from(values.keySet());
    }

    public Boolean has(String column) {
        return lookup(column).isDefined();
    }

    public Result<Object, DbError> value(String column) {
        var found = nullableValue(column);
        if (found instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }
        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<Option<Object>, DbError>) found;
        var option = ok.value();
        if (option instanceof Option.None<?>) {
            return Jdbc.err("SQL column '" + column + "' is null");
        }
        @SuppressWarnings("unchecked")
        var some = (Option.Some<Object>) option;
        return Jdbc.ok(some.value());
    }

    public Result<Option<Object>, DbError> nullableValue(String column) {
        var key = lookup(column);
        if (key instanceof Option.None<?>) {
            return Jdbc.err("SQL row has no column '" + column + "'");
        }
        @SuppressWarnings("unchecked")
        var some = (Option.Some<String>) key;
        var value = values.get(some.value());
        if (value == null) {
            return Jdbc.ok(LumeRuntime.optionNone());
        }
        return Jdbc.ok(LumeRuntime.optionSome(value));
    }

    public <T> Result<T, DbError> decode(LumeType targetType) {
        var prepared = JdbcRowDecoder.prepare(targetType);
        if (prepared instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }
        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<JdbcRowDecoder, DbError>) prepared;
        return ok.value().decode(this);
    }

    public Result<String, DbError> str(String column) {
        return convert(column, value -> value instanceof String text ? text : String.valueOf(value));
    }

    public Result<Option<String>, DbError> strOpt(String column) {
        return convertOpt(column, value -> value instanceof String text ? text : String.valueOf(value));
    }

    public Result<Long, DbError> intValue(String column) {
        return convert(column, JdbcRow::toLong);
    }

    public Result<Long, DbError> int64(String column) {
        return intValue(column);
    }

    public Result<Integer, DbError> int32(String column) {
        return convert(column, value -> Math.toIntExact(toLong(value)));
    }

    public Result<Option<Long>, DbError> intOpt(String column) {
        return convertOpt(column, JdbcRow::toLong);
    }

    public Result<Double, DbError> floatValue(String column) {
        return convert(column, JdbcRow::toDouble);
    }

    public Result<Double, DbError> float64(String column) {
        return floatValue(column);
    }

    public Result<Float, DbError> float32(String column) {
        return convert(column, value -> toDouble(value).floatValue());
    }

    public Result<Option<Double>, DbError> floatOpt(String column) {
        return convertOpt(column, JdbcRow::toDouble);
    }

    public Result<Boolean, DbError> bool(String column) {
        return convert(column, JdbcRow::toBool);
    }

    public Result<Option<Boolean>, DbError> boolOpt(String column) {
        return convertOpt(column, JdbcRow::toBool);
    }

    Object decodeField(LumeField field, Class<?> paramType) throws DecodeFailure {
        var column = field.name();
        var key = lookup(column);
        if (key instanceof Option.None<?>) {
            throw new DecodeFailure("SQL row has no column '" + column + "' for field '" + field.name() + "'");
        }

        @SuppressWarnings("unchecked")
        var some = (Option.Some<String>) key;
        var value = values.get(some.value());
        if (value == null) {
            if (isOptionType(field.fieldType()) || Option.class.isAssignableFrom(paramType)) {
                return LumeRuntime.optionNone();
            }
            throw new DecodeFailure("SQL column '" + column + "' is null for non-optional field '"
                    + field.name() + "'");
        }

        try {
            if (isOptionType(field.fieldType()) || Option.class.isAssignableFrom(paramType)) {
                return LumeRuntime.optionSome(convertByDescriptor(value, optionInnerDescriptor(field.fieldType())));
            }
            return convertForJava(value, paramType, field.fieldType());
        } catch (RuntimeException err) {
            throw new DecodeFailure("SQL column '" + column + "' cannot be converted for field '"
                    + field.name() + "': " + err.getMessage());
        }
    }

    private static Object convertForJava(Object value, Class<?> paramType, LumeType fieldType) {
        if (paramType == Object.class) {
            return convertByDescriptor(value, typeName(fieldType));
        }
        if (paramType == String.class) {
            return String.valueOf(value);
        }
        if (paramType == Long.class || paramType == Long.TYPE) {
            return toLong(value);
        }
        if (paramType == Integer.class || paramType == Integer.TYPE) {
            return Math.toIntExact(toLong(value));
        }
        if (paramType == Double.class || paramType == Double.TYPE) {
            return toDouble(value);
        }
        if (paramType == Float.class || paramType == Float.TYPE) {
            return toDouble(value).floatValue();
        }
        if (paramType == Boolean.class || paramType == Boolean.TYPE) {
            return toBool(value);
        }
        if (paramType.isInstance(value)) {
            return value;
        }
        return convertByDescriptor(value, typeName(fieldType));
    }

    private static Object convertByDescriptor(Object value, String descriptor) {
        return switch (descriptor) {
            case "Str" -> String.valueOf(value);
            case "Int", "Int64" -> toLong(value);
            case "Int32", "Rune" -> Math.toIntExact(toLong(value));
            case "Float", "Float64" -> toDouble(value);
            case "Float32" -> toDouble(value).floatValue();
            case "Bool" -> toBool(value);
            default -> value;
        };
    }

    private static boolean isOptionType(LumeType type) {
        var name = typeName(type);
        return name.equals("Option") || name.startsWith("Option[");
    }

    private static String optionInnerDescriptor(LumeType type) {
        var name = typeName(type);
        if (name.startsWith("Option[") && name.endsWith("]")) {
            return name.substring("Option[".length(), name.length() - 1).trim();
        }
        return "Any";
    }

    private static String typeName(LumeType type) {
        var name = type.name();
        return name.isDefined() ? name.orPanic() : type.toString();
    }

    static String typeLabel(LumeType type) {
        var qualified = type.qualifiedName();
        if (qualified.isDefined()) {
            return qualified.orPanic();
        }
        return typeName(type);
    }

    private <T> Result<T, DbError> convert(String column, Converter<T> converter) {
        var raw = value(column);
        if (raw instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }
        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<Object, DbError>) raw;
        try {
            return Jdbc.ok(converter.convert(ok.value()));
        } catch (RuntimeException err) {
            return Jdbc.err("SQL column '" + column + "' cannot be converted: " + err.getMessage());
        }
    }

    private <T> Result<Option<T>, DbError> convertOpt(String column, Converter<T> converter) {
        var raw = nullableValue(column);
        if (raw instanceof Result.Err<?, ?> err) {
            @SuppressWarnings("unchecked")
            var error = (DbError) err.error();
            return new Result.Err<>(error);
        }
        @SuppressWarnings("unchecked")
        var ok = (Result.Ok<Option<Object>, DbError>) raw;
        var option = ok.value();
        if (option instanceof Option.None<?>) {
            return Jdbc.ok(LumeRuntime.optionNone());
        }
        @SuppressWarnings("unchecked")
        var some = (Option.Some<Object>) option;
        try {
            return Jdbc.ok(LumeRuntime.optionSome(converter.convert(some.value())));
        } catch (RuntimeException err) {
            return Jdbc.err("SQL column '" + column + "' cannot be converted: " + err.getMessage());
        }
    }

    private Option<String> lookup(String column) {
        if (values.containsKey(column)) {
            return LumeRuntime.optionSome(column);
        }
        var expected = column.toLowerCase(Locale.ROOT);
        for (var key : values.keySet()) {
            if (key.toLowerCase(Locale.ROOT).equals(expected)) {
                return LumeRuntime.optionSome(key);
            }
        }
        return LumeRuntime.optionNone();
    }

    private static Long toLong(Object value) {
        if (value instanceof Number number) {
            return number.longValue();
        }
        if (value instanceof String text) {
            return Long.parseLong(text);
        }
        throw new IllegalArgumentException(value.getClass().getSimpleName());
    }

    private static Double toDouble(Object value) {
        if (value instanceof Number number) {
            return number.doubleValue();
        }
        if (value instanceof String text) {
            return Double.parseDouble(text);
        }
        throw new IllegalArgumentException(value.getClass().getSimpleName());
    }

    private static Boolean toBool(Object value) {
        if (value instanceof Boolean bool) {
            return bool;
        }
        if (value instanceof Number number) {
            return number.longValue() != 0L;
        }
        if (value instanceof String text) {
            var normalized = text.trim().toLowerCase(Locale.ROOT);
            return switch (normalized) {
                case "true", "t", "yes", "y", "1" -> true;
                case "false", "f", "no", "n", "0" -> false;
                default -> throw new IllegalArgumentException(text);
            };
        }
        throw new IllegalArgumentException(value.getClass().getSimpleName());
    }

    @FunctionalInterface
    private interface Converter<T> {
        T convert(Object value);
    }

    static final class DecodeFailure extends Exception {
        DecodeFailure(String message) {
            super(message);
        }
    }
}
