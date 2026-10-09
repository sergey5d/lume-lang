package lume.core;

import java.util.LinkedHashSet;
import java.util.Set;
import java.util.function.Function;

public final class LumeSet<T> {
    private final LinkedHashSet<T> values;

    private LumeSet(LinkedHashSet<T> values) {
        this.values = values;
    }

    public static <T> LumeSet<T> empty() {
        return new LumeSet<>(new LinkedHashSet<>());
    }

    public static <T> LumeSet<T> from(Iterable<T> values) {
        var set = new LinkedHashSet<T>();
        for (var value : values) {
            set.add(value);
        }
        return new LumeSet<>(set);
    }

    public LumeSet<T> add(T value) {
        values.add(value);
        return this;
    }

    public LumeSet<T> addAll(LumeSet<T> other) {
        values.addAll(other.values);
        return this;
    }

    public boolean contains(T value) {
        return values.contains(value);
    }

    public long size() {
        return values.size();
    }

    public boolean isEmpty() {
        return values.isEmpty();
    }

    public boolean nonEmpty() {
        return !values.isEmpty();
    }

    public String makeStr(String separator) {
        return makeStr(separator, String::valueOf);
    }

    public String makeStr(String separator, Function<? super T, String> render) {
        return String.join(separator, values.stream().map(render).toList());
    }

    public Set<T> asJava() {
        return Set.copyOf(values);
    }
}
