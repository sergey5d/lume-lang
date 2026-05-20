# EXPECT:
# tuple 5 hehe
# tuple inferred 6 there
# tuple mixed 8 mixed
# tuple mixed2 9 mixed2
# tuple pair 20 pair
# tuple skip only unused
# tuple skip 14 xxx
# 0

def main() Int {
    a Int, b Str = (5, "hehe")
    OS.println("tuple", a, b)

    inferredTupleLeft, inferredTupleRight = (6, "there")
    OS.println("tuple inferred", inferredTupleLeft, inferredTupleRight)

    mixedTupleLeft Int, mixedTupleRight = (8, "mixed")
    OS.println("tuple mixed", mixedTupleLeft, mixedTupleRight)

    mixedTuple2Left Int, mixedTuple2Right Str = (9, "mixed2")
    OS.println("tuple mixed2", mixedTuple2Left, mixedTuple2Right)

    tuplePairLeft, tuplePairRight = (20, "pair")
    OS.println("tuple pair", tuplePairLeft, tuplePairRight)

    _, skippedOnlyTupleValue = (21, "unused")
    OS.println("tuple skip only", skippedOnlyTupleValue)

    skippedTupleLeft Int, _, skippedTupleRight Str = (14, "drop", "xxx")
    OS.println("tuple skip", skippedTupleLeft, skippedTupleRight)

    0
}
