# EXPECT:
# sum 3
# grouped 14
# size 4
# match 20
# def 7
# if 11
# for 5
# match stmt 9
# total 76

def helper() Int =
    3 +
        4

def main() Unit {
    sum Int = 1 +
        2

    grouped Int = (
        3
        + 4
    ) *
        2

    items List[Int] = [
        1,
        2,
        3
    ]

    size Int = "haha".
        size()

    matched Int = match 2 {
        case 1 =>
            10
        case 2 =>
            20
        case _ =>
            0
    }

    fromIf Int = if true then 11 else 0

    collected = for item <- [2, 3] yield item

    fromFor Int = collected.get(0).getOr(0) +
        collected.get(1).getOr(0)

    var fromMatchStmt = 0
    match 2 {
        case 1 => {
            fromMatchStmt := 1
        }
    }

    match 3 {
        case 3 => {
            fromMatchStmt := 9
        }
    }

    helperValue Int = helper()

    total Int = sum +
        grouped +
        items.size() +
        size +
        matched +
        helperValue +
        fromIf +
        fromFor +
        fromMatchStmt

    OS.println("sum", sum)
    OS.println("grouped", grouped)
    OS.println("size", size)
    OS.println("match", matched)
    OS.println("def", helperValue)
    OS.println("if", fromIf)
    OS.println("for", fromFor)
    OS.println("match stmt", fromMatchStmt)
    OS.println("total", total)
}
