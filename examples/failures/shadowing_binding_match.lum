# FAIL_REGEX:
# .*binding 'value' shadows an existing variable; use a different name.*

enum MaybeInt {
    case SomeX {
        value Int
    }

    case NoneX
}

def main() Int {
    value = 3
    maybe = MaybeInt.SomeX(1)
    match maybe {
        case SomeX(value) => value
        case NoneX => value
    }
}
