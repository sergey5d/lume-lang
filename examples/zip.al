# EXPECT:
# list zip 1 a
# list zip 2 b
# list zipWithIndex 3 2
# array zip 5 y
# array zipWithIndex 6 2
# range zip 11 q
# range zipWithIndex 8 1
# pair sum 3
# indexed sum 4
# array pair sum 5
# range pair sum 21
# range index sum 16

def main() Unit {
    items = List(1, 2, 3)
    listPairs = items.zip(List("a", "b"))
    listIndexed = items.zipWithIndex()

    unwrap firstPair <- listPairs.get(0) else {
        ()
    }
    unwrap secondPair <- listPairs.get(1) else ()

    unwrap indexedPair <- listIndexed.get(2) else ()

    firstLeft, firstRight = firstPair
    secondLeft, secondRight = secondPair
    indexedValue, indexedPos = indexedPair

    OS.println("list zip", firstLeft, firstRight)
    OS.println("list zip", secondLeft, secondRight)
    OS.println("list zipWithIndex", indexedValue, indexedPos)

    values = Array.ofLength(3)
    values[0] := 4
    values[1] := 5
    values[2] := 6
    other = Array.ofLength(2)
    other[0] := "x"
    other[1] := "y"
    arrayPairs = values.zip(other)
    arrayIndexed = values.zipWithIndex()

    arrayLeft, arrayRight = arrayPairs[1]
    arrayIndexedValue, arrayIndexedPos = arrayIndexed[2]

    OS.println("array zip", arrayLeft, arrayRight)
    OS.println("array zipWithIndex", arrayIndexedValue, arrayIndexedPos)

    rangePairs = Range(10, 13).zip(List("p", "q"))
    rangeIndexed = Range(7, 9).zipWithIndex()

    unwrap rangePair <- rangePairs.get(1) else {
        ()
    }
    unwrap rangeIndexedPair <- rangeIndexed.get(1) else {
        ()
    }
    rangeLeft, rangeRight = rangePair
    rangeIndexedValue, rangeIndexedPos = rangeIndexedPair

    OS.println("range zip", rangeLeft, rangeRight)
    OS.println("range zipWithIndex", rangeIndexedValue, rangeIndexedPos)

    var pairSum = 0
    for left, right <- listPairs {
        if right == "a" || right == "b" {
            pairSum += left
        }
    }
    OS.println("pair sum", pairSum)

    var indexedSum = 0
    for value, index <- listIndexed {
        if index < 2 {
            indexedSum += value + index
        }
    }
    OS.println("indexed sum", indexedSum)

    var arrayPairSum = 0
    for left, right <- arrayPairs {
        if right == "y" {
            arrayPairSum += left
        }
    }
    OS.println("array pair sum", arrayPairSum)

    var rangePairSum = 0
    for left, right <- rangePairs {
        if right == "p" || right == "q" {
            rangePairSum += left
        }
    }
    OS.println("range pair sum", rangePairSum)

    var rangeIndexSum = 0
    for value, index <- rangeIndexed {
        rangeIndexSum += value + index
    }
    OS.println("range index sum", rangeIndexSum)
}
