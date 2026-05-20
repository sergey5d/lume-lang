# EXPECT:
# explicit
# implicit

interface ExplicitPrinter {
    def printExplicit() Unit
}

interface ImplicitPrinter {
    def printImplicit()
}

class Printer with ExplicitPrinter, ImplicitPrinter {
}

impl Printer {
    def printExplicit() {
        OS.println("explicit")
    }

    def printImplicit() {
        OS.println("implicit")
    }
}

def main() Unit {
    printer Printer = Printer()
    printer.printExplicit()
    printer.printImplicit()
}
