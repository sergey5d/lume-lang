# EXPECT:
# 1
# 2
# 9

enum BoolBox {
    case Wrap {
        value Bool
    }
    case Empty
}

enum PairBox {
    case Full {
        value (Int, Int)
    }
    case NoneX
}

def describeFlag(value BoolBox) Int =
    match value {
        case Wrap(true) => 1
        case Wrap(false) => 2
        case BoolBox.Empty => 0
    }

def describePair(value PairBox) Int =
    match value {
        case Full((left, right)) => left + right
        case PairBox.NoneX => 0
    }

def main() Unit {
    OS.println(describeFlag(BoolBox.Wrap(true)))
    OS.println(describeFlag(BoolBox.Wrap(false)))
    OS.println(describePair(PairBox.Full((4, 5))))
}
