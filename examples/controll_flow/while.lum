# EXPECT:
# block 0
# block 1
# block 2
# short 0
# short 1
# short 2
# 0

def next(label Str, value Int) Int {
    OS.println(label + " " + value)
    value + 1
}

def main() Int {
    var count Int = 0
    while count < 3 {
        OS.println("block " + count)
        count += 1
    }

    var shortCount Int = 0
    while shortCount < 3 {
        shortCount := next("short", shortCount)
    }

    0
}
