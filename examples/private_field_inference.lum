# EXPECT:
# box 3
# box2 7
# object Hello 2
# 0

class CounterBox {
    hidden count = 1
    hidden var hits = 0
}

impl CounterBox {
    def bump() Unit {
        this.hits += this.count
    }

    def total() Int = this.hits
}

object Greeter {
    hidden hello = "Hello"
}

impl Greeter {
    def greet(times Int) Str = this.hello + " " + times
}

def main() Int {
    box = CounterBox()
    box.bump()
    box.bump()
    box.bump()
    OS.println("box " + box.total())

    box2 = CounterBox()
    box2.bump()
    box2.bump()
    box2.bump()
    box2.bump()
    box2.bump()
    box2.bump()
    box2.bump()
    OS.println("box2 " + box2.total())

    OS.println("object " + Greeter.greet(2))
    0
}
