# EXPECT:
# override class
# conflict first

interface Greeter {
    def greet() Str = "interface"
}

class OverrideBox with Greeter {
}

impl OverrideBox {
    def greet() Str = "class"
}

interface FirstChoice {
    def choose() Str = "first"
}

interface SecondChoice {
    def choose() Str = "second"
}

class PreferFirst with FirstChoice, SecondChoice {
}

def main() Unit {
    greeter Greeter = OverrideBox()
    prefer = PreferFirst()

    OS.println("override", greeter.greet())
    OS.println("conflict", prefer.choose())
}
