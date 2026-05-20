# EXPECT:
# left: bad
# right: num 7
# left
# right

enum Outcome {
    tag Str

    case Left {
        value Str
        tag = "left"
    }

    case Right {
        value Int
        tag = "right"
    }
}

impl Outcome {
    def describe() Str = match this {
        case Left(value) => "left: " + value
        case Right(value) => "right: num " + value
    }
}

def main() Unit {
    left Outcome = Outcome.Left("bad")
    right Outcome = Outcome.Right(7)

    OS.println(left.describe())
    OS.println(right.describe())
    OS.println(left.tag)
    OS.println(right.tag)
}
