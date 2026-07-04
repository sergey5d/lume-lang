package lume.core;

import java.util.LinkedHashSet;
import java.util.Set;

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

    public Set<T> asJava() {
        return Set.copyOf(values);
    }
}
