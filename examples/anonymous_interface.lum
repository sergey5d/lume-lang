# EXPECT:
# x
# closed
# solo

interface Reader {
    def read() Str
}

interface Closer {
    def close() Unit
}

def main() Unit {
    handler = Reader with Closer {
        def read() Str = "x"
        def close() Unit = OS.println("closed")
    }

    single = Reader {
        def read() Str = "solo"
    }

    OS.println(handler.read())
    handler.close()
    OS.println(single.read())
}
