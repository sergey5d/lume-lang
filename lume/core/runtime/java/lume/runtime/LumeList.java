package lume.runtime;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class LumeList<T> {
    private final ArrayList<T> values;

    private LumeList(ArrayList<T> values) {
        this.values = values;
    }

    public static <T> LumeList<T> empty() {
        return new LumeList<>(new ArrayList<>());
    }

    @SafeVarargs
    public static <T> LumeList<T> of(T... values) {
        var list = new ArrayList<T>();
        Collections.addAll(list, values);
        return new LumeList<>(list);
    }

    public static <T> LumeList<T> from(Iterable<T> values) {
        var list = new ArrayList<T>();
        for (var value : values) {
            list.add(value);
        }
        return new LumeList<>(list);
    }

    public long size() {
        return values.size();
    }

    public Option<T> get(long index) {
        if (index < 0 || index >= values.size()) {
            return Option.none();
        }
        return Option.some(values.get((int) index));
    }

    public LumeList<T> add(T value) {
        var copy = new ArrayList<>(values);
        copy.add(value);
        return new LumeList<>(copy);
    }

    public LumeList<T> addAll(LumeList<T> other) {
        var copy = new ArrayList<>(values);
        copy.addAll(other.values);
        return new LumeList<>(copy);
    }

    public List<T> asJava() {
        return List.copyOf(values);
    }
}
