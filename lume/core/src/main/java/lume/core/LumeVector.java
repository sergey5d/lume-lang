package lume.core;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

public final class LumeVector<T> {
    private final ArrayList<T> values;

    private LumeVector(ArrayList<T> values) {
        this.values = values;
    }

    public static <T> LumeVector<T> empty() {
        return new LumeVector<>(new ArrayList<>());
    }

    @SafeVarargs
    public static <T> LumeVector<T> of(T... values) {
        var list = new ArrayList<T>();
        Collections.addAll(list, values);
        return new LumeVector<>(list);
    }

    public static <T> LumeVector<T> from(Iterable<T> values) {
        var list = new ArrayList<T>();
        for (var value : values) {
            list.add(value);
        }
        return new LumeVector<>(list);
    }

    public long size() {
        return values.size();
    }

    public Option<T> get(long index) {
        if (index < 0 || index >= values.size()) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(values.get((int) index));
    }

    public LumeVector<T> add(T value) {
        values.add(value);
        return this;
    }

    public void set(long index, T value) {
        values.set(Math.toIntExact(index), value);
    }

    @SuppressWarnings("unchecked")
    public LumeVector<T> addAll(Object other) {
        var iterator = LumeIterator.<T>from(other);
        while (iterator.hasNext()) {
            values.add((T) iterator.next());
        }
        return this;
    }

    public LumeVector<Tuple2<T, Long>> zipWithIndex() {
        var indexed = new ArrayList<Tuple2<T, Long>>(values.size());
        for (var index = 0; index < values.size(); index++) {
            indexed.add(new Tuple2<>(values.get(index), (long) index));
        }
        return new LumeVector<>(indexed);
    }

    public LumeIterator<T> iterator() {
        return LumeIterator.from(this);
    }

    public List<T> asJava() {
        return List.copyOf(values);
    }
}
