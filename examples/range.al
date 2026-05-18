# EXPECT:
# range 1
# range 2
# range 3
# total 6
# another range 10
# another range 11
# another range 12
# another range 13
# another multiplied 17160
# descending 5
# descending 4
# descending 3
# descending 2
# descending total 14
# stepped 10
# stepped 8
# stepped 6
# stepped total 24

def main() Unit {
    var total Int = 0
    for item <- (1, 4) {
        OS.println("range", item)
        total := total + item
    }
    OS.println("total", total)

    var multiplied = 1
    for item <- (10, 14) {
        OS.println("another range", item)
        multiplied *= item
    }
    OS.println("another multiplied", multiplied)

    var descendingTotal = 0
    for item <- (5, 1) {
        OS.println("descending", item)
        descendingTotal += item
    }
    OS.println("descending total", descendingTotal)

    var steppedTotal = 0
    for item <- Range.create(10, 4, -2) {
        OS.println("stepped", item)
        steppedTotal += item
    }
    OS.println("stepped total", steppedTotal)
}
