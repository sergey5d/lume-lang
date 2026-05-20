# EXPECT:
# item 2
# item 4
# item 6
# doubled 2 6
# expanded 1 13
# filtered 2 3
# fold 6
# reduce 6
# exists true
# forAll true
# 0

def main() Int {
    items List[Int] = List(1, 2, 3)
    doubled List[Int] = items.map((item Int) -> item * 2)
    doubled2 List[Int] = items.map(item -> item * 2)
    expanded List[Int] = items.flatMap((item Int) -> List(item, item + 10))
    filtered List[Int] = items.filter((item Int) -> item > 1)
    total Int = items.fold(0, (acc Int, item Int) -> acc + item)
    reduced Option[Int] = items.reduce((left Int, right Int) -> left + right)
    hasBig Bool = items.exists((item Int) -> item > 2)
    allPositive Bool = items.forAll((item Int) -> item > 0)

    doubled.forEach((item Int) -> OS.println("item " + item))

    OS.println("doubled " + doubled.get(0).getOr(0) + " " + doubled.get(2).getOr(0))
    OS.println("expanded " + expanded.get(0).getOr(0) + " " + expanded.get(5).getOr(0))
    OS.println("filtered " + filtered.get(0).getOr(0) + " " + filtered.get(1).getOr(0))
    OS.println("fold " + total)
    OS.println("reduce " + reduced.getOr(0))
    OS.println("exists " + hasBig)
    OS.println("forAll " + allPositive)
    0
}
