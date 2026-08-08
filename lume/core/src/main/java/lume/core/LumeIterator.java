package lume.core;

import java.util.Iterator;

public final class LumeIterator<T> {
    private final Iterator<?> iterator;

    private LumeIterator(Iterator<?> iterator) {
        this.iterator = iterator;
    }

    @SuppressWarnings("unchecked")
    public static <T> LumeIterator<T> from(Object source) {
        if (source instanceof LumeIterator<?> lumeIterator) {
            return (LumeIterator<T>) lumeIterator;
        }
        if (source instanceof Option<?> option) {
            return option.isDefined()
                    ? new LumeIterator<>(
                            java.util.List.of(LumeRuntime.extractSuccessValue(option)).iterator())
                    : new LumeIterator<>(java.util.List.of().iterator());
        }
        if (source instanceof LumeVector<?> list) {
            return new LumeIterator<>(list.asJava().iterator());
        }
        if (source instanceof LumeLinkedList<?> list) {
            return new LumeIterator<>(list.asJava().iterator());
        }
        if (source instanceof LumeSet<?> set) {
            return new LumeIterator<>(set.asJava().iterator());
        }
        if (source instanceof LumeArray<?> array) {
            return new LumeIterator<>(array.asJava().iterator());
        }
        if (source instanceof Iterable<?> iterable) {
            return new LumeIterator<>(iterable.iterator());
        }
        throw new IllegalArgumentException("value is not iterable: " + source);
    }

    public boolean hasNext() {
        return iterator.hasNext();
    }

    @SuppressWarnings("unchecked")
    public T next() {
        return (T) iterator.next();
    }
}
