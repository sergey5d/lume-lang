package lume.core;

import java.util.Collections;
import java.util.function.BiFunction;
import java.util.function.Function;

public final class LumeLinkedList<T> {
    private final java.util.LinkedList<T> values;

    private LumeLinkedList(java.util.LinkedList<T> values) {
        this.values = values;
    }

    public static <T> LumeLinkedList<T> empty() {
        return new LumeLinkedList<>(new java.util.LinkedList<>());
    }

    @SafeVarargs
    public static <T> LumeLinkedList<T> of(T... values) {
        var list = new java.util.LinkedList<T>();
        Collections.addAll(list, values);
        return new LumeLinkedList<>(list);
    }

    public static <T> LumeLinkedList<T> from(Iterable<T> values) {
        var list = new java.util.LinkedList<T>();
        for (var value : values) {
            list.add(value);
        }
        return new LumeLinkedList<>(list);
    }

    public LumeLinkedList<T> append(T value) {
        values.addLast(value);
        return this;
    }

    public LumeLinkedList<T> add(T value) {
        values.addLast(value);
        return this;
    }

    @SuppressWarnings("unchecked")
    public LumeLinkedList<T> addAll(Object source) {
        var iterator = LumeIterator.<T>from(source);
        while (iterator.hasNext()) {
            values.addLast((T) iterator.next());
        }
        return this;
    }

    public <X> LumeLinkedList<X> map(Function<? super T, ? extends X> mapper) {
        var result = LumeLinkedList.<X>empty();
        for (var value : values) {
            result.add(mapper.apply(value));
        }
        return result;
    }

    public <X> LumeLinkedList<X> flatMap(Function<? super T, LumeLinkedList<X>> mapper) {
        var result = LumeLinkedList.<X>empty();
        for (var value : values) {
            result.addAll(mapper.apply(value));
        }
        return result;
    }

    public LumeLinkedList<T> filter(Function<? super T, Boolean> predicate) {
        var result = LumeLinkedList.<T>empty();
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

    public <X> X reduce(X initial, BiFunction<X, T, X> reducer) {
        return fold(initial, reducer);
    }

    public Option<T> reduce(BiFunction<T, T, T> reducer) {
        if (values.isEmpty()) {
            return LumeRuntime.optionNone();
        }
        var iterator = values.iterator();
        var result = iterator.next();
        while (iterator.hasNext()) {
            result = reducer.apply(result, iterator.next());
        }
        return LumeRuntime.optionSome(result);
    }

    public boolean exists(Function<? super T, Boolean> predicate) {
        for (var value : values) {
            if (Boolean.TRUE.equals(predicate.apply(value))) {
                return true;
            }
        }
        return false;
    }

    public boolean forAll(Function<? super T, Boolean> predicate) {
        for (var value : values) {
            if (!Boolean.TRUE.equals(predicate.apply(value))) {
                return false;
            }
        }
        return true;
    }

    public <X> LumeLinkedList<Tuple2<T, X>> zip(LumeLinkedList<X> other) {
        var result = LumeLinkedList.<Tuple2<T, X>>empty();
        var left = values.iterator();
        var right = other.values.iterator();
        while (left.hasNext() && right.hasNext()) {
            result.add(new Tuple2<>(left.next(), right.next()));
        }
        return result;
    }

    public LumeLinkedList<Tuple2<T, Long>> zipWithIndex() {
        var result = LumeLinkedList.<Tuple2<T, Long>>empty();
        long index = 0;
        for (var value : values) {
            result.add(new Tuple2<>(value, index));
            index++;
        }
        return result;
    }

    public Option<T> at(long index) {
        if (index < 0 || index >= values.size()) {
            return LumeRuntime.optionNone();
        }
        return LumeRuntime.optionSome(values.get(Math.toIntExact(index)));
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

    public Option<T> head() {
        return first();
    }

    public Option<T> first() {
        return values.isEmpty() ? LumeRuntime.optionNone() : LumeRuntime.optionSome(values.getFirst());
    }

    public Option<T> last() {
        return values.isEmpty() ? LumeRuntime.optionNone() : LumeRuntime.optionSome(values.getLast());
    }

    public LumeLinkedList<T> tail() {
        if (values.isEmpty()) {
            return empty();
        }
        return from(values.subList(1, values.size()));
    }

    public boolean isEmpty() {
        return values.isEmpty();
    }

    public boolean nonEmpty() {
        return !values.isEmpty();
    }

    public Option<T> removeFirst() {
        return values.isEmpty() ? LumeRuntime.optionNone() : LumeRuntime.optionSome(values.removeFirst());
    }

    public Option<T> removeLast() {
        return values.isEmpty() ? LumeRuntime.optionNone() : LumeRuntime.optionSome(values.removeLast());
    }

    public long size() {
        return values.size();
    }

    public void forEach(Function<? super T, LumeUnit> action) {
        for (var value : values) {
            action.apply(value);
        }
    }

    public LumeIterator<T> iterator() {
        return LumeIterator.from(this);
    }

    public String makeStr(String separator) {
        return String.join(separator, values.stream().map(String::valueOf).toList());
    }

    public java.util.List<T> asJava() {
        return java.util.List.copyOf(values);
    }
}
