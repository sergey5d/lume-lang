# EXPECT:
# 3578
# 246
# 1
# 3578
# 246
# 1
# 3578
# 246
# 1
# 3578
# 246
# 1

object RowOrdering with Ordering[(Int, Int, Str)] {
    def compare(left (Int, Int, Str), right (Int, Int, Str)) Int {
        leftX Int, leftY Int, _ = left
        rightX Int, rightY Int, _ = right

        if leftY > rightY {
            return -1
        }
        if leftY < rightY {
            return 1
        }
        if leftX < rightX {
            return -1
        }
        if leftX > rightX {
            return 1
        }
        return 0
    }
}

def syntax1(rows List[(Int, Int, Str)]) Unit {
    var lastX = 0
    if _, initialY, _ <- rows.get(0) {
        var lastY = initialY
        for x Int, y Int, char Str <- rows {
            for lineStep <- Range(y, lastY) {
                OS.println()
            }
            if lastX < x {
                for spaceStep <- Range(lastX, x) {
                    OS.print(" ")
                }
            }
            OS.print(char)
            lastX := x + 1
            lastY := y
        }

        OS.println()
    }
}

def syntax2(rows List[(Int, Int, Str)]) Unit {
    var lastX = 0
    if _, initialY Int, _ <- rows.get(0) {
        var lastY = initialY
        for x, y, char <- rows {
            for lineStep <- Range(y, lastY) {
                OS.println()
            }
            if lastX < x {
                for spaceStep <- Range(lastX, x) {
                    OS.print(" ")
                }
            }
            OS.print(char)
            lastX := x + 1
            lastY := y
        }

        OS.println()
    }
}

def syntax3(rows List[(Int, Int, Str)]) Unit {
    var lastX = 0

    if first <- rows.get(0) {
        _, initialY Int, _ = first
        var lastY = initialY
        for row <- rows {
            x, y, char Str = row
            for lineStep <- Range(y, lastY) {
                OS.println()
            }
            if lastX < x {
                for spaceStep <- Range(lastX, x) {
                    OS.print(" ")
                }
            }
            OS.print(char)
            lastX := x + 1
            lastY := y
        }

        OS.println()
    }
}

def syntax4(rows List[(Int, Int, Str)]) Unit {
    var lastX = 0
    for {
        _, initialY Int, _ <- rows.get(0)
        var lastY = initialY
        x, y, char Str <- rows
        # just as an example
        xImmutable = x
    } yield {
        for lineStep <- Range(y, lastY) {
            OS.println()
        }
        if lastX < xImmutable {
            for spaceStep <- Range(lastX, xImmutable) {
                OS.print(" ")
            }
        }
        OS.print(char)
        lastX := x + 1
        lastY := y
        char
    }
    OS.println()
}

def main() Unit {
    rows List[(Int, Int, Str)] = [
        (0, 0, "1"),
        (0, 1, "2"),
        (0, 2, "3"),
        (1, 1, "4"),
        (1, 2, "5"),
        (2, 1, "6"),
        (2, 2, "7"),
        (3, 2, "8")
    ]
    rows.sort(RowOrdering)

    syntax1(rows)
    syntax2(rows)
    syntax3(rows)
    syntax4(rows)
}
