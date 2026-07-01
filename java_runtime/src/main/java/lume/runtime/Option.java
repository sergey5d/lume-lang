package lume.runtime;

public sealed interface Option<T> permits Option.Some, Option.None {
    boolean isDefined();

    T orPanic();

    static <T> Option<T> some(T value) {
        return new Some<>(value);
    }

    static <T> Option<T> none() {
        return new None<>();
    }

    record Some<T>(T value) implements Option<T> {
        @Override
        public boolean isDefined() {
            return true;
        }

        @Override
        public T orPanic() {
            return value;
        }
    }

    record None<T>() implements Option<T> {
        @Override
        public boolean isDefined() {
            return false;
        }

        @Override
        public T orPanic() {
            throw new LumePanic("expected Option.Some");
        }
    }
}
