# EXPECT:
# reddish: false
# some defined: true
# none: true

# Enums might have 2 flavors.
# Fully defined where all attributes have values.

enum Color {
    color Str
    temperature Int

    def isReddish() Bool = temperature % 5 == 0

    case Black {
        color = "xxx"
        temperature = 1
    }
    case Red {
        color = "xxx2"
        temperature = 2
    }
}

# Naive implementation of option type
enum OptionX[T] {

    def isDefined() Bool = this != None

    case NoneX
    case SomeX {
        value T
    }
}

black = Color.Black
reddish = black.isReddish()

someInt = OptionX.SomeX(5)
noneInt = OptionX.NoneX()

someDefined = someInt.isDefined()

def main() Unit {
    OS.println("reddish:", reddish)
    OS.println("some defined:", someDefined)
    OS.println("none:", noneInt == OptionX.NoneX)
}
