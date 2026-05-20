# EXPECT:
# 8
# 10
# 2
# 0
# 2
# true

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def main() Unit {
    values = List(1, 6, 3)
    ifMapped = values.map(if _ > 5 then 10 else 8)

    options = List(MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3))
    matchMapped = options.map(match {
       case SomeX(x) => x + 1
       case NoneX => 0
    })
    partialMapped List[Option[Int]] = options.map(partial {
       case SomeX(x) => x + 1
    })

    OS.println(ifMapped.get(0).getOr(0))
    OS.println(ifMapped.get(1).getOr(0))
    OS.println(matchMapped.get(0).getOr(0))
    OS.println(matchMapped.get(1).getOr(0))
    unwrap firstPartial <- partialMapped.get(0) else ()
    unwrap secondPartial <- partialMapped.get(1) else ()
    OS.println(firstPartial.getOr(0))
    OS.println(secondPartial.isEmpty())
}
