package lume.db;

import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;

import lume.core.LumeList;
import lume.core.LumeRuntime;
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
}
