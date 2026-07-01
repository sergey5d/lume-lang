package lume.runtime;

import java.util.Iterator;
import java.util.NoSuchElementException;

public final class Range implements Iterable<Long> {
    private final long start;
    private final long end;

    public Range(long start, long end) {
        this.start = start;
        this.end = end;
    }

    @Override
    public Iterator<Long> iterator() {
        return new Iterator<>() {
            private long next = start;

            @Override
            public boolean hasNext() {
                return next < end;
            }

            @Override
            public Long next() {
                if (!hasNext()) {
                    throw new NoSuchElementException();
                }
                return next++;
            }
        };
    }
}
