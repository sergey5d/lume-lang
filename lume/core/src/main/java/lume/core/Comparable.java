package lume.core;

/** A value with a semantic ordering relative to another value of type T. */
public interface Comparable<T> {
    Long compareTo(T other);
}
