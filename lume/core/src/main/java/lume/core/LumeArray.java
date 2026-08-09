package lume.core;

import java.util.Arrays;
import java.util.List;
import java.util.function.IntFunction;
import java.util.function.Supplier;

public final class LumeArray<T> {
    private final Object[] values;

    private LumeArray(Object[] values) {
        this.values = values;
    }

    public static LumeArray<Long> ofInt(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, 0L);
        return new LumeArray<>(values);
    }

    public static LumeArray<Double> ofFloat(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, 0.0);
        return new LumeArray<>(values);
    }

    public static LumeArray<Boolean> ofBool(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, false);
        return new LumeArray<>(values);
    }

    public static LumeArray<String> ofStr(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, "");
        return new LumeArray<>(values);
    }

    public static LumeArray<Integer> ofRune(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, 0);
        return new LumeArray<>(values);
    }

    public static <T> LumeArray<T> fill(long length, T value) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, value);
        return new LumeArray<>(values);
    }

    public static <T> LumeArray<T> fill(long length, Supplier<T> supplier) {
        var values = new Object[Math.toIntExact(length)];
        for (int i = 0; i < values.length; i++) {
            values[i] = supplier.get();
        }
        return new LumeArray<>(values);
    }

    public static <T> LumeArray<T> generate(long length, IntFunction<T> supplier) {
        var values = new Object[Math.toIntExact(length)];
        for (int i = 0; i < values.length; i++) {
            values[i] = supplier.apply(i);
        }
        return new LumeArray<>(values);
    }

    public long size() {
        return values.length;
    }

    public Option<T> at(long index) {
        if (index < 0 || index >= values.length) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(valueAt(index));
    }

    public Result<T, InvalidIndex> setAt(long index, T value) {
        if (index < 0 || index >= values.length) {
            return new Result.Err<>(new InvalidIndex(index, values.length));
        }
        var previous = valueAt(index);
        values[Math.toIntExact(index)] = value;
        return new Result.Ok<>(previous);
    }

    public void set(long index, T value) {
        values[Math.toIntExact(index)] = value;
    }

    @SuppressWarnings("unchecked")
    public List<T> asJava() {
        return (List<T>) Arrays.asList(values);
    }

    @SuppressWarnings("unchecked")
    private T valueAt(long index) {
        return (T) values[Math.toIntExact(index)];
    }
}
