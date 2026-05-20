# EXPECT:
# red
# palette

enum Color {
    def label() Str = match this {
        case Color.Red => "red"
        case Color.Blue => "blue"
    }

    case Red
    case Blue
}

object Color {
    def palette() Str = "palette"
}

def main() Unit {
    color Color = Color.Red
    OS.println(color.label())
    OS.println(Color.palette())
}
