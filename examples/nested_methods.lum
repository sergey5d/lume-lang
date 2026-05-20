# EXPECT:
# 15
# 16
# 31
# 0

def main() Int {
    base = 10

    def add(left Int, right Int) Int {
        left + right + base
    }

    def double(value Int) Int = value * 2

    OS.println(add(2, 3))
    OS.println(double(8))

    def combine(value Int) Int {
        def bump(amount Int) Int = amount + 1
        add(bump(value), double(5))
    }

    OS.println(combine(10))
    0
}
