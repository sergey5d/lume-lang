# EXPECT:
# hop
# jump 3
# true
# true

interface Hopper {
    def hop() Str
}

interface Jumper {
    def jump(steps Int) Str
}

class Rabbit with Hopper, Jumper {
}

impl Rabbit {
    def hop() Str = "hop"

    def jump(steps Int) Str = "jump " + steps
}

def main() Unit {
    rabbit = Rabbit()
    OS.println(rabbit.hop())
    OS.println(rabbit.jump(3))
    OS.println(rabbit is Hopper)
    OS.println(rabbit is Jumper)
}
