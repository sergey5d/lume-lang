package alang.stdlib;

import java.util.Iterator;

public final class IntRange implements Iterable<Long> {
    public final long start;
    public final long end;
    public final long step;

    public IntRange(long start, long end) {
        this(start, end, start < end ? 1L : -1L);
    }

    public IntRange(long start, long end, long step) {
        this.start = start;
        this.end = end;
        this.step = step;
    }

    @Override
    public Iterator<Long> iterator() {
        return new Iterator<>() {
            private long current = start;

            @Override
            public boolean hasNext() {
                if (step > 0L) {
                    return current < end;
                }
                return current > end;
            }

            @Override
            public Long next() {
                long value = current;
                current += step;
                return value;
            }
        };
    }

    public <X> List<Tuple2<Long, X>> zip(List<X> other) {
        List<Tuple2<Long, X>> out = List.of();
        Iterator<X> right = other.iterator();
        for (Long left : this) {
            if (!right.hasNext()) {
                return out;
            }
            out.append(new Tuple2<>(left, right.next()));
        }
        return out;
    }

    public List<Tuple2<Long, Long>> zipWithIndex() {
        List<Tuple2<Long, Long>> out = List.of();
        long index = 0L;
        for (Long item : this) {
            out.append(new Tuple2<>(item, index));
            index += 1L;
        }
        return out;
    }
}
