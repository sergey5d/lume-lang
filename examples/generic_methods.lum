# EXPECT:
# 15
# value=7
# 0

def id[T](value T) T = value

class Mapper {
}

impl Mapper {
    def map[X](value Int, fn Int -> X) X {
        fn(value)
    }
}

def main() Int {
    mapper Mapper = Mapper()
    OS.println(mapper.map(5, (x Int) -> x + 10))
    OS.println(id(mapper.map(7, (x Int) -> "value=" + x)))
    0
}
