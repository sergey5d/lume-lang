package lume.core;

import java.math.RoundingMode;

/** Explicit decimal rounding; no ambient precision or rounding context. */
public sealed interface Rounding permits Rounding.Up, Rounding.Down, Rounding.Ceiling,
        Rounding.Floor, Rounding.HalfUp, Rounding.HalfDown, Rounding.HalfEven, Rounding.Unnecessary {
    RoundingMode mode();

    final class Up implements Rounding {
        public static final Up INSTANCE = new Up();
        private Up() {}
        public RoundingMode mode() { return RoundingMode.UP; }
        @Override public String toString() { return "Up"; }
    }
    final class Down implements Rounding {
        public static final Down INSTANCE = new Down();
        private Down() {}
        public RoundingMode mode() { return RoundingMode.DOWN; }
        @Override public String toString() { return "Down"; }
    }
    final class Ceiling implements Rounding {
        public static final Ceiling INSTANCE = new Ceiling();
        private Ceiling() {}
        public RoundingMode mode() { return RoundingMode.CEILING; }
        @Override public String toString() { return "Ceiling"; }
    }
    final class Floor implements Rounding {
        public static final Floor INSTANCE = new Floor();
        private Floor() {}
        public RoundingMode mode() { return RoundingMode.FLOOR; }
        @Override public String toString() { return "Floor"; }
    }
    final class HalfUp implements Rounding {
        public static final HalfUp INSTANCE = new HalfUp();
        private HalfUp() {}
        public RoundingMode mode() { return RoundingMode.HALF_UP; }
        @Override public String toString() { return "HalfUp"; }
    }
    final class HalfDown implements Rounding {
        public static final HalfDown INSTANCE = new HalfDown();
        private HalfDown() {}
        public RoundingMode mode() { return RoundingMode.HALF_DOWN; }
        @Override public String toString() { return "HalfDown"; }
    }
    final class HalfEven implements Rounding {
        public static final HalfEven INSTANCE = new HalfEven();
        private HalfEven() {}
        public RoundingMode mode() { return RoundingMode.HALF_EVEN; }
        @Override public String toString() { return "HalfEven"; }
    }
    final class Unnecessary implements Rounding {
        public static final Unnecessary INSTANCE = new Unnecessary();
        private Unnecessary() {}
        public RoundingMode mode() { return RoundingMode.UNNECESSARY; }
        @Override public String toString() { return "Unnecessary"; }
    }
}
