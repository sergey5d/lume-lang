# EXPECT:
# some 5
# none
# 5
# 100
# left-right
# 0

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

enum PairText {
    case PairX {
        left Str
        right Str
    }
}

def printOption(value MaybeInt) {
    match value {
        case SomeX(x) => {
            OS.println("some " + x)
        }
        case MaybeInt.NoneX => {
            OS.println("none")
        }
    }
}

def matchSingleLine(value MaybeInt) Int = match value {
    case SomeX(x) => x
    case MaybeInt.NoneX => 100
}

def main() Int {
    some MaybeInt = MaybeInt.SomeX(5)
    none MaybeInt = MaybeInt.NoneX()
    printOption(some)
    printOption(none)
    OS.println(matchSingleLine(some))
    OS.println(matchSingleLine(none))

    pairText PairText = PairText.PairX("left", "right")
    OS.println(match pairText {
        case PairX(left, right) => left + "-" + right
    })

    0
}
