# SKIP

interface List[T] with Iterable[T] {
    def append(value T) List[T]
    def map[X](f T -> X) List[X]
    def flatMap[X](f T -> List[X]) List[X]
    def filter(f T -> Bool) List[T]
    def fold[X](initial X, f (X, T) -> X) X
    def reduce(f (T, T) -> T) Option[T]
    def exists(f T -> Bool) Bool
    def forAll(f T -> Bool) Bool
    def sort(ordering Ordering[T]) List[T]
    def zip[X](other List[X]) List[(T, X)]
    def zipWithIndex() List[(T, Int)]
    def get(index Int) Option[T]
    def head() Option[T]
    def tail() List[T]
    def isEmpty() Bool
    def remove(index Int) Option[T]
    def removeLast() Option[T]
    def size() Int
    def forEach(f T -> Unit)
}
