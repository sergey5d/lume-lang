package lume.core;

/** A value with a semantic ordering relative to another value of type T. */
public interface Ordered<T> {
    Long compare(T other);
}
