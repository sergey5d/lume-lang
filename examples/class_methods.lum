# EXPECT:
# 12
# 0

class Adder {
    amount Int
}

impl Adder {
    def add(value Int) Int = amount + value
}

def main() Int {
    adder Adder = Adder(5)
    OS.println(adder.add(7))
    0
}
