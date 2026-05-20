# FAIL_REGEX:
# invalid_match_pattern at .*: runtime type patterns cannot specify generic arguments; use the erased outer type

class Box[T] {
    value T
}

def main() Option[Int] =
    partial Box(7) {
        case _ Box[Int] => 1
    }
