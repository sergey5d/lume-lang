# EXPECT:
# original 10 x
# updated 42 x

record Amount {
    amount Int
    description Str
}

def main() Unit {
    value = Amount(10, "x")
    updated = value with { amount = 42 }

    OS.println("original " + value.amount + " " + value.description)
    OS.println("updated " + updated.amount + " " + updated.description)
}
