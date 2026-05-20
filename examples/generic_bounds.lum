# EXPECT:
# 7
# 0

object Ascending with Ordering[Int] {
    def compare(left Int, right Int) Int = left - right
}

class Box[T with Ordering[T]] {
    value T
}

class Mapper {
}

impl Mapper {
    def pick[X with Ordering[X]](value X) X = value
}

def pick[T with Ordering[T]](value T) T = value
def useBox(value Box[Int]) Int = value.value

def main() Int {
    mapper Mapper = Mapper()
    OS.println(pick(3) + mapper.pick(4))
    0
}
