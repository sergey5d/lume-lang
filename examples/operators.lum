# EXPECT:
# 4 6
# -4
# 5
# 3

class Vec {
    hidden var items Array[Int]
}

impl Vec {
    def init(left Int, right Int) {
        this.items := Array.ofLength(2)
        this.items[0] := left
        this.items[1] := right
    }

    def [](index Int) Int = items[index]
    def +(other Vec) Vec = Vec(this[0] + other[0], this[1] + other[1])
    def -() Vec = Vec(-this[0], -this[1])
}

def main() Unit {
    left Vec = Vec(1, 2)
    right Vec = Vec(3, 4)
    total Vec = left + right
    neg Vec = -total

    OS.println(total[0], total[1])
    OS.println(neg[0])

    items = List(1, 2)
    items2 = items :+ 3
    merged = items2 ++ List(4, 5)
    OS.println(merged[4])

    seen = Set(1, 2)
    all = seen ++ Set(3)
    OS.println(all.size())
}
