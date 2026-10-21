package lume.core;

import java.util.Iterator;
import java.util.NoSuchElementException;

public final class IntRange implements Iterable<Long> {
    public final long start;
    public final long end;
    public final long step;

    public IntRange(long start, long end) {
        this(start, end, 1);
    }

    public IntRange(long start, long end, long step) {
        if (step == 0) {
            throw new LumePanic("Range step cannot be zero");
        }
        this.start = start;
        this.end = end;
        this.step = step;
    }

    @Override
    public LumeIterator<Long> iterator() {
        Iterable<Long> values = () -> new Iterator<>() {
            private long current = start;
            private boolean exhausted;

            @Override
            public boolean hasNext() {
                return !exhausted && (step >= 0 ? current < end : current > end);
            }

            @Override
            public Long next() {
                if (!hasNext()) {
                    throw new NoSuchElementException();
                }
                var value = current;
                // Stepping beyond Int's range exhausts the range, not the program.
                try {
                    current = Math.addExact(current, step);
                } catch (ArithmeticException error) {
                    exhausted = true;
                }
                return value;
            }
        };
        return LumeIterator.from(values);
    }

    public <X> LumeVector<Tuple2<Long, X>> zip(LumeVector<X> other) {
        var result = LumeVector.<Tuple2<Long, X>>empty();
        var left = iterator();
        var right = other.iterator();
        while (left.hasNext() && right.hasNext()) {
            result.add(new Tuple2<>(left.next(), right.next()));
        }
        return result;
    }

    public LumeVector<Tuple2<Long, Long>> zipWithIndex() {
        var result = LumeVector.<Tuple2<Long, Long>>empty();
        var values = iterator();
        long index = 0;
        while (values.hasNext()) {
            result.add(new Tuple2<>(values.next(), index));
            index += 1;
        }
        return result;
    }
}
