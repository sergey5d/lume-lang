# EXPECT:
# count 3
# item 10
# item 30
# item 50

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def expandPage(page List[MaybeInt]) List[Int] {
    extracted List[Int] = []

    page.forEach(maybeValue -> {
        parsed Option[Int] = partial maybeValue {
           case SomeX(x) if x > 0 => {
                x * 10
            }
        }

        if parsed.isSet() {
            extracted.append(parsed.expect())
        }
    })

    extracted
}

def main() Unit {
    pages List[List[MaybeInt]] = [
        [ MaybeInt.SomeX(1), MaybeInt.NoneX, MaybeInt.SomeX(3) ],
        List(MaybeInt.NoneX, MaybeInt.SomeX(5))
    ]

    extracted List[Int] = pages.flatMap(page -> expandPage(page))

    OS.println("count " + extracted.size())
    extracted.forEach(item -> OS.println("item " + item))
}
