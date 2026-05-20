# EXPECT:
# each 4
# each 5
# each 6
# index 4 6
# get 5 true
# first 4
# last 6
# mapped 8 12
# exists true
# forAll true
# zip 5 b
# zipIndex 6 2
# assigned 9 10
# size 3

def main() Unit {
    values Array[Int] = Array(4, 5, 6)
    labels Array[Str] = Array("a", "b")

    mapped = values.map(item -> item * 2)
    hasBig = values.exists(item -> item > 5)
    allPositive = values.forAll(item -> item > 0)

    values.forEach(item -> OS.println("each " + item))
    OS.println("index " + values[0] + " " + values[2])

    unwrap middle <- values.get(1) else ()
    unwrap first <- values.first() else ()
    unwrap last <- values.last() else ()

    pairs = values.zip(labels)
    indexed = values.zipWithIndex()

    zippedValue, zippedLabel = pairs[1]
    indexedValue, indexedPos = indexed[2]

    assigned = Array.ofLength(2)
    assigned[0] := 9
    assigned[1] := 10

    OS.println("get " + middle + " " + values.get(9).isEmpty())
    OS.println("first " + first)
    OS.println("last " + last)
    OS.println("mapped " + mapped[0] + " " + mapped[2])
    OS.println("exists " + hasBig)
    OS.println("forAll " + allPositive)
    OS.println("zip " + zippedValue + " " + zippedLabel)
    OS.println("zipIndex " + indexedValue + " " + indexedPos)
    OS.println("assigned " + assigned[0] + " " + assigned[1])
    OS.println("size " + values.size())
}
