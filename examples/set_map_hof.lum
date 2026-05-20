# EXPECT:
# doubled true 3
# expanded true 6
# filtered true 2
# setFold 6
# setReduce 6
# setExists true
# setForAll true
# set 1
# set 2
# set 3
# mapped 10 20
# mappedValues 100 200
# expandedValues 1 12
# filteredMap true 1
# mapFold 3
# mapReduce b 2
# mapExists true
# mapForAll true
# pair a 1
# pair b 2
# total 9

def main() {
    seen = Set(1, 2, 3)

    doubled = seen.map(item -> item * 2)
    OS.println("doubled " + doubled.contains(4) + " " + doubled.size())

    expanded = seen.flatMap(item -> Set(item, item + 10))
    OS.println("expanded " + expanded.contains(12) + " " + expanded.size())

    filtered = seen.filter(item -> item > 1)
    OS.println("filtered " + filtered.contains(2) + " " + filtered.size())

    setTotal = seen.fold(0, (acc Int, item) -> acc + item)
    OS.println("setFold " + setTotal)

    setReduced = seen.reduce((left, right) -> left + right)
    OS.println("setReduce " + setReduced.getOr(0))

    setHasBig = seen.exists(item -> item > 2)
    OS.println("setExists " + setHasBig)

    setAllPositive = seen.forAll(item -> item > 0)
    OS.println("setForAll " + setAllPositive)

    seen.forEach(item -> OS.println("set " + item))

    values = Map("a" : 1, "b" : 2)

    mapped = values.map((key, value) -> value * 10)
    OS.println("mapped " + mapped.get(0).getOr(0) + " " + mapped.get(1).getOr(0))

    mappedValues = values.mapValues(value -> value * 100)
    OS.println("mappedValues " + mappedValues["a"].getOr(0) + " " + mappedValues["b"].getOr(0))

    expandedValues = values.flatMap((key, value) -> List(value, value + 10))
    OS.println("expandedValues " + expandedValues.get(0).getOr(0) + " " + expandedValues.get(3).getOr(0))

    filteredMap = values.filter((key, value) -> value > 1)
    OS.println("filteredMap " + filteredMap.contains("b") + " " + filteredMap.size())

    mapTotal = values.fold(0, (acc, key, value) -> acc + value)
    OS.println("mapFold " + mapTotal)

    reducedPairOpt = values.reduce((leftKey, leftVal, rightKey, rightVal) -> (rightKey, rightVal))

    unwrap reducedPair <- reducedPairOpt else ()

    reducedKey, reducedValue = reducedPair
    OS.println("mapReduce " + reducedKey + " " + reducedValue)

    mapHasB = values.exists((key, value) -> key == "b")
    OS.println("mapExists " + mapHasB)

    mapAllSmall = values.forAll((key, value) -> value < 3)
    OS.println("mapForAll " + mapAllSmall)

    values.forEach((key, value) -> OS.println("pair " + key + " " + value))

    var total = 0

    for item Int <- seen {
        total += item
    }

    for key Str, value Int <- values {
        total += value
    }

    OS.println("total " + total)
}
