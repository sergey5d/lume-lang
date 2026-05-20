# EXPECT:
# each 1
# each 2
# each 3
# each 4
# index 1 4
# mapped 2 8
# expanded 1 14
# filtered 3 4
# fold 10
# reduce 10
# exists true
# forAll true
# sorted 4 1
# zip 2 b
# zipIndex 4 3
# get 3
# head 1
# tail 2 4
# remove 2 3
# size 3

object Descending with Ordering[Int] {
    def compare(left Int, right Int) Int = right - left
}

def main() Unit {
    items = [1, 2, 3]
    items.append(4)

    mapped = items.map(item -> item * 2)
    expanded = items.flatMap(item -> [item, item + 10])
    filtered = items.filter(item -> item > 2)
    total = items.fold(0, (acc, item) -> acc + item)
    reduced = items.reduce((left, right) -> left + right)
    hasBig = items.exists(item -> item > 3)
    allPositive = items.forAll(item -> item > 0)

    items.forEach(item -> OS.println("each " + item))
    OS.println("index " + items[0] + " " + items[3])

    sorted = [3, 1, 4, 2]
    sorted.sort(Descending)

    zipped = items.zip(["a", "b"])
    indexed = items.zipWithIndex()
    tailItems = items.tail()

    unwrap zippedPair <- zipped.get(1) else ()
    unwrap indexedPair <- indexed.get(3) else ()
    unwrap head <- items.head() else ()
    unwrap removed <- items.remove(1) else ()

    zipValue, zipLabel = zippedPair
    indexedValue, indexedPos = indexedPair

    OS.println("mapped " + mapped.get(0).getOr(0) + " " + mapped.get(3).getOr(0))
    OS.println("expanded " + expanded.get(0).getOr(0) + " " + expanded.get(7).getOr(0))
    OS.println("filtered " + filtered.get(0).getOr(0) + " " + filtered.get(1).getOr(0))
    OS.println("fold " + total)
    OS.println("reduce " + reduced.getOr(0))
    OS.println("exists " + hasBig)
    OS.println("forAll " + allPositive)
    OS.println("sorted " + sorted.get(0).getOr(0) + " " + sorted.get(3).getOr(0))
    OS.println("zip " + zipValue + " " + zipLabel)
    OS.println("zipIndex " + indexedValue + " " + indexedPos)
    OS.println("get " + items.get(1).getOr(0))
    OS.println("head " + head)
    OS.println("tail " + tailItems.get(0).getOr(0) + " " + tailItems.get(2).getOr(0))
    OS.println("remove " + removed + " " + items.size())
    OS.println("size " + items.size())
}
