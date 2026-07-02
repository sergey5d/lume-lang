package lume.runtime;

public sealed interface Result<T, E> permits Result.Ok, Result.Err {
    boolean isOk();

    T orPanic();

    static <T, E> Result<T, E> ok(T value) {
        return new Ok<>(value);
    }

    static <T, E> Result<T, E> err(E error) {
        return new Err<>(error);
    }

    record Ok<T, E>(T value) implements Result<T, E> {
        @Override
        public boolean isOk() {
            return true;
        }

        @Override
        public T orPanic() {
            return value;
        }
    }

    record Err<T, E>(E error) implements Result<T, E> {
        @Override
        public boolean isOk() {
            return false;
        }

        @Override
        public T orPanic() {
            throw new LumePanic("expected Result.Ok");
        }
    }
}
