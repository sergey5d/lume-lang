package lume.core;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.function.BiFunction;
import java.util.function.Function;

public final class LumeVector<T> implements Iterable<T> {
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

    public boolean contains(T value) {
        return values.contains(value);
    }

    public Option<T> at(long index) {
        if (index < 0 || index >= values.size()) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(values.get((int) index));
    }

    public Option<T> first() {
        return values.isEmpty()
            ? LumeRuntime.optionNone()
            : LumeRuntime.optionSome(values.get(0));
    }

    public Option<T> last() {
        return values.isEmpty()
            ? LumeRuntime.optionNone()
            : LumeRuntime.optionSome(values.get(values.size() - 1));
    }

    public LumeVector<T> slice() {
        return slice(0, values.size());
    }

    public LumeVector<T> slice(long start) {
        return slice(start, values.size());
    }

    public LumeVector<T> slice(long start, long end) {
        if (start < 0 || end < start || end > values.size()) {
            throw new LumePanic(
                "Vector.slice range "
                    + start
                    + ":"
                    + end
                    + " is out of bounds for size "
                    + values.size()
            );
        }
        return new LumeVector<>(
            new ArrayList<>(values.subList(Math.toIntExact(start), Math.toIntExact(end)))
        );
    }

    public Result<T, InvalidIndex> setAt(long index, T value) {
        if (index < 0 || index >= values.size()) {
            return new Result.Err<>(new InvalidIndex(index, values.size()));
        }
        return new Result.Ok<>(values.set(Math.toIntExact(index), value));
    }

    public Result<LumeUnit, InvalidIndex> insertAt(long index, T value) {
        if (index < 0 || index > values.size()) {
            return new Result.Err<>(new InvalidIndex(index, values.size()));
        }
        values.add(Math.toIntExact(index), value);
        return new Result.Ok<>(LumeUnit.INSTANCE);
    }

    public Result<T, InvalidIndex> removeAt(long index) {
        if (index < 0 || index >= values.size()) {
            return new Result.Err<>(new InvalidIndex(index, values.size()));
        }
        return new Result.Ok<>(values.remove(Math.toIntExact(index)));
    }

    public LumeVector<T> add(T value) {
        values.add(value);
        return this;
    }

    public <X> LumeVector<X> map(Function<? super T, ? extends X> mapper) {
        var result = LumeVector.<X>empty();
        for (var value : values) {
            result.add(mapper.apply(value));
        }
        return result;
    }

    public <X> LumeVector<X> flatMap(Function<? super T, ?> mapper) {
        var result = LumeVector.<X>empty();
        for (var value : values) {
            result.addAll(mapper.apply(value));
        }
        return result;
    }

    public <X> LumeVector<X> flatten() {
        var result = LumeVector.<X>empty();
        for (var value : values) {
            result.addAll(value);
        }
        return result;
    }

    public LumeVector<T> filter(Function<? super T, Boolean> predicate) {
        var result = LumeVector.<T>empty();
        for (var value : values) {
            if (Boolean.TRUE.equals(predicate.apply(value))) {
                result.add(value);
            }
        }
        return result;
    }

    public <X> X fold(X initial, BiFunction<X, T, X> reducer) {
        var result = initial;
        for (var value : values) {
            result = reducer.apply(result, value);
        }
        return result;
    }

    public LumeVector<T> sort(BiFunction<T, T, Long> compare) {
        values.sort((left, right) -> Long.compare(compare.apply(left, right), 0));
        return this;
    }

    @SuppressWarnings("unchecked")
    public LumeVector<T> sort() {
        values.sort((left, right) ->
            Long.compare(((Ordered<T>) left).compare(right), 0)
        );
        return this;
    }

    public LumeVector<T> take(long count) {
        var end = Math.min(Math.max(count, 0), values.size());
        return new LumeVector<>(new ArrayList<>(values.subList(0, Math.toIntExact(end))));
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

    public String makeStr(String separator) {
        return makeStr(separator, String::valueOf);
    }

    public String makeStr(String separator, Function<? super T, String> render) {
        return String.join(separator, values.stream().map(render).toList());
    }

    @Override
    public LumeIterator<T> iterator() {
        return LumeIterator.from(this);
    }

    public List<T> asJava() {
        return List.copyOf(values);
    }
}
