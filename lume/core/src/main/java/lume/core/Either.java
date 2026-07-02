package lume.core;

public sealed interface Either<L, R> permits Either.Left, Either.Right {
    boolean isRight();

    R orPanic();

    static <L, R> Either<L, R> left(L value) {
        return new Left<>(value);
    }

    static <L, R> Either<L, R> right(R value) {
        return new Right<>(value);
    }

    record Left<L, R>(L value) implements Either<L, R> {
        @Override
        public boolean isRight() {
            return false;
        }

        @Override
        public R orPanic() {
            throw new LumePanic("expected Either.Right");
        }
    }

    record Right<L, R>(R value) implements Either<L, R> {
        @Override
        public boolean isRight() {
            return true;
        }

        @Override
        public R orPanic() {
            return value;
        }
    }
}
