package lume.core;

/** Lume values with semantic equality and a compatible stable hash. */
public interface Hashed<T> extends Eq<T> {
    default Long hash() {
        return (long) hashCode();
    }
}
