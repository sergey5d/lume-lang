package lume.core;

import java.util.Iterator;

final class LumeIterator {
    private final Iterator<?> iterator;

    private LumeIterator(Iterator<?> iterator) {
        this.iterator = iterator;
    }

    static LumeIterator from(Object source) {
        if (source instanceof LumeIterator lumeIterator) {
            return lumeIterator;
        }
        if (source instanceof LumeList<?> list) {
            return new LumeIterator(list.asJava().iterator());
        }
        if (source instanceof LumeSet<?> set) {
            return new LumeIterator(set.asJava().iterator());
        }
        if (source instanceof LumeArray<?> array) {
            return new LumeIterator(array.asJava().iterator());
        }
        if (source instanceof Iterable<?> iterable) {
            return new LumeIterator(iterable.iterator());
        }
        throw new IllegalArgumentException("value is not iterable: " + source);
    }

    boolean hasNext() {
        return iterator.hasNext();
    }

    Object next() {
        return iterator.next();
    }
}
