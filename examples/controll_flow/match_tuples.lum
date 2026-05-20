# EXPECT:
# exact tuple
# exact tuple 2-3
# exact tuple 4-5
# tuple 1 2
# 0

def main() Int {
    exactPair = match (1, 2) {
        case (1, 2) => "exact tuple"
        case _ => "miss"
    }
    OS.println(exactPair)

    exactPair2 = match (2, 3) {
        case (a, b) => "exact tuple " + a + "-" + b
        case _ => "miss"
    }
    OS.println(exactPair2)

    exactPair3 = match (4, 5) {
        case (a, b) => "exact tuple " + a + "-" + b
        case _ => "miss"
    }
    OS.println(exactPair3)

    pair = (1, 2)
    match pair {
        case (left, right) => {
            OS.println("tuple " + left + " " + right)
        }
    }

    0
}
