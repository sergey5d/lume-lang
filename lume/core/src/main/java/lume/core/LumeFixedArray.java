package lume.core;

import java.util.Arrays;
import java.util.List;
import java.util.function.IntFunction;
import java.util.function.Supplier;

public final class LumeFixedArray<T> {
    private final Object[] values;

    private LumeFixedArray(Object[] values) {
        this.values = values;
    }

    public static LumeFixedArray<Long> ofInt(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, 0L);
        return new LumeFixedArray<>(values);
    }

    public static LumeFixedArray<Double> ofFloat(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, 0.0);
        return new LumeFixedArray<>(values);
    }

    public static LumeFixedArray<Boolean> ofBool(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, false);
        return new LumeFixedArray<>(values);
    }

    public static LumeFixedArray<String> ofStr(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, "");
        return new LumeFixedArray<>(values);
    }

    public static LumeFixedArray<Integer> ofRune(long length) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, 0);
        return new LumeFixedArray<>(values);
    }

    public static <T> LumeFixedArray<T> fill(long length, T value) {
        var values = new Object[Math.toIntExact(length)];
        Arrays.fill(values, value);
        return new LumeFixedArray<>(values);
    }

    public static <T> LumeFixedArray<T> fill(long length, Supplier<T> supplier) {
        var values = new Object[Math.toIntExact(length)];
        for (int i = 0; i < values.length; i++) {
            values[i] = supplier.get();
        }
        return new LumeFixedArray<>(values);
    }

    public static <T> LumeFixedArray<T> generate(long length, IntFunction<T> supplier) {
        var values = new Object[Math.toIntExact(length)];
        for (int i = 0; i < values.length; i++) {
            values[i] = supplier.apply(i);
        }
        return new LumeFixedArray<>(values);
    }

    public long size() {
        return values.length;
    }

    public Option<T> get(long index) {
        if (index < 0 || index >= values.length) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(valueAt(index));
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
