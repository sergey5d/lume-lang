# EXPECT:
# yield 2
# yield 3
# yield 4
# size 3
# 0

def main() Int {
    items = for item <- [1, 2, 3] yield item + 1

    for item <- items {
        OS.println("yield " + item)
    }

    OS.println("size " + items.size())
    0
}
