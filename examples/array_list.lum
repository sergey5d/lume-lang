# SKIP: generic class construction is not supported at call sites yet
#
# Intended future example for a generic array-backed list once calls like
# ArrayList[Int](5) and ArrayListIterator[T](...) are supported.

class ArrayListIterator[T] with Iterator[T] {
    hidden var items Array[T]
    hidden var limit Int
    hidden var index Int = 0
}

impl ArrayListIterator[T] {
    def init(items Array[T], limit Int) {
        this.items := items
        this.limit := limit
    }

    def hasNext() Bool = index < limit

    def next() T {
        value T = items[index]
        index := index + 1
        value
    }
}

class ArrayList[T] with Iterable[T] {
    hidden var items Array[T]
    hidden var count Int = 0
}

impl ArrayList[T] {
    def init(capacity Int) {
        this.items := Array.ofLength(capacity)
    }

    def append(value T) Unit {
        items[count] := value
        count := count + 1
    }

    def get(index Int) Option[T] =
        if index < 0 || index >= count {
            None()
        } else {
            Some(items[index])
        }

    def size() Int = count

    def iterator() Iterator[T] = ArrayListIterator[T](items = items, limit = count)
}

def main() Int {
    values ArrayList[Int] = ArrayList[Int](5)
    values.append(10)
    values.append(20)
    values.append(30)

    var sum = 0
    for item <- values {
        sum := sum + item
    }

    OS.println("size " + values.size())
    OS.println("sum " + sum)
    0
}
