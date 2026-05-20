# EXPECT:
# hop
# jump 3
# true
# true
# true

interface Hopper {
    def hop() Str
}

interface Jumper {
    def jump(steps Int) Str
}

interface Acrobat with Hopper, Jumper {
}

class Rabbit with Acrobat {
}

impl Rabbit {
    def hop() Str = "hop"

    def jump(steps Int) Str = "jump " + steps
}

def main() Unit {
    rabbit = Rabbit()
    OS.println(rabbit.hop())
    OS.println(rabbit.jump(3))
    OS.println(rabbit is Acrobat)
    OS.println(rabbit is Hopper)
    OS.println(rabbit is Jumper)
}
