# EXPECT:
# hop
# jump 2
# 0

interface Hopper {
    def hop() Str = "hop"
}

interface Jumper {
    def jump(steps Int) Str
}

class Rabbit with Hopper, Jumper {
}

impl Rabbit {
    def jump(steps Int) Str = "jump " + steps
}

def main() Int {
    rabbit Hopper = Rabbit()
    jumper Jumper = Rabbit()
    OS.println(rabbit.hop())
    OS.println(jumper.jump(2))
    0
}
