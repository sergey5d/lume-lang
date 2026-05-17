# SKIP

interface Iterator[T] {
    def hasNext() Bool
    def next() T
}

interface Iterable[T] {
    def iterator() Iterator[T]
}
