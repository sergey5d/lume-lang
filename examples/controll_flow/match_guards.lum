# EXPECT:
# 12
# 10
# 0

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def describe(value MaybeInt) Int =
    match value {
        case SomeX(x) if x > 10 => x
        case SomeX(_) => 10
        case MaybeInt.NoneX => 0
    }

def main() Unit {
    large MaybeInt = MaybeInt.SomeX(12)
    small MaybeInt = MaybeInt.SomeX(3)
    empty MaybeInt = MaybeInt.NoneX()

    OS.println(describe(large))
    OS.println(describe(small))
    OS.println(describe(empty))
}
