# PRELUDE_SKIP
# JAVA_SKIP
# INTERPRETER

interface Array[T] with Iterable[T] {

    # TODO: think more about whether [] should stay declared here
    # or only exist as language-level operator surface.
    # def [](idx Int) T
    # def [](idx Int, val T)

    def get(idx Int) Option[T]

    def first() Option[T]
    def last() Option[T]

    def clone() Array[T]

    def map[X](f T -> X) Array[X]
    def forEach(f T -> Unit)

    def count(f T -> Bool) Int

    def exists(f T -> Bool) Bool
    def forAll(f T -> Bool) Bool

    def contains(e T) Bool
    def find(e T) Option[T]

    def indexOf(e T) Int

    # TODO: think more about whether these belong on Array directly.
    # Arrays have fixed size, so the result shape is less obvious here.
    # def filter(f T -> Bool) List[T]
    # def flatMap[X](f T -> Iterable[X]) List[X]

    def zip[X](other Array[X]) Array[(T, X)]
    def zipWithIndex() Array[(T, Int)]

    def size() Int
}
