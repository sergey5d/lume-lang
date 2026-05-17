# PRELUDE_SKIP
# JAVA_SKIP
# INTERPRETER

interface Iterator[T] {
    def hasNext() Bool
    def next() T
}

interface Iterable[T] {
    def iterator() Iterator[T]
}
