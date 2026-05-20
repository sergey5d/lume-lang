# EXPECT:
# then
# else-if
# picked 5
# binding 7
# 0

def main() Int {
    if true {
        OS.println("then")
    } else {
        OS.println("else")
    }

    if false {
        OS.println("nope")
    } else if true {
        OS.println("else-if")
    } else {
        OS.println("also nope")
    }

    picked = if false {
        1
    } else {
        5
    }
    OS.println("picked " + picked)

    values = [7]
    if value <- values.get(0) {
        OS.println("binding " + value)
    } else {
        OS.println("binding none")
    }

    0
}
