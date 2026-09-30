package lume.core;

import java.util.function.BiFunction;

public interface Ordering<T> extends BiFunction<T, T, Long> {
    Long compare(T left, T right);

    @Override
    default Long apply(T left, T right) {
        return compare(left, right);
    }
}
