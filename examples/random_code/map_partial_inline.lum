# EXPECT:
# first 20
# second true
# third 60

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def main() Unit {
    values List[MaybeInt] = List(
        MaybeInt.SomeX(2),
        MaybeInt.NoneX,
        MaybeInt.SomeX(6)
    )

    extracted List[Option[Int]] = values.map(partial _ {
           case SomeX(x) => x * 10
        }
    )

    unwrap first <- extracted.get(0) else ()
    unwrap second <- extracted.get(1) else ()
    unwrap third <- extracted.get(2) else ()

    OS.println("first " + first.getOr(0))
    OS.println("second " + second.isEmpty())
    OS.println("third " + third.getOr(0))
}
