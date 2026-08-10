# FAIL_REGEX:
# non_exhaustive_match at .*: match does not cover enum cases: NoneX

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def main() Int =
    match MaybeInt.SomeX(5) {
        case SomeX(x) => x
    }
