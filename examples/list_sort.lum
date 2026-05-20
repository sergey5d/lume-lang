# EXPECT:
# asc 1 2 3 4
# desc 4 3 2 1
# 0

object Ascending with Ordering[Int] {
    def compare(left Int, right Int) Int = left - right
}

object Descending with Ordering[Int] {
    def compare(left Int, right Int) Int = right - left
}

def main() Int {
    asc List[Int] = List(3, 1, 4, 2)
    asc.sort(Ascending)
    OS.println("asc " + asc.get(0).getOr(0) + " " + asc.get(1).getOr(0) + " " + asc.get(2).getOr(0) + " " + asc.get(3).getOr(0))

    desc List[Int] = List(3, 1, 4, 2)
    desc.sort(Descending)
    OS.println("desc " + desc.get(0).getOr(0) + " " + desc.get(1).getOr(0) + " " + desc.get(2).getOr(0) + " " + desc.get(3).getOr(0))

    0
}
