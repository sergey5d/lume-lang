# EXPECT:
# xxx
# 8
# 6
# 20
# 1
# 20
# 42

enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def main() Unit {
    a1 = {
        1 + 7
    }
    {
        OS.println("xxx")
    }
    var v = {
        a = 5
        {
            a + 1
        }
    }
    var branch = {
        {
            if false {
                10
            } else {
                20
            }
        }
    }
    yielded = {
        for item <- [1, 2, 3] yield {
            if item % 2 == 0 {
                item * 10
            } else {
                item
            }
        }
    }
    matched = {
        match MaybeInt.SomeX(42) {
            case SomeX(value) => value
            case MaybeInt.NoneX => 0
        }
    }

    OS.println(a1)
    OS.println(v)
    OS.println(branch)
    OS.println(yielded.get(0).getOr(0))
    OS.println(yielded.get(1).getOr(0))
    OS.println(matched)
}
