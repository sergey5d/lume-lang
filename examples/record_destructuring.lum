# EXPECT:
# record 7 world
# record inferred 10 infer
# record mixed 11 partial
# record pair 22 record-pair
# record skip only visible
# record skip 15 kept
# 0

record Pair {
    left Int
    right Str
}

record Triple {
    first Int
    middle Str
    last Str
}

def main() Int {
    c Int, d Str = Pair(7, "world")
    OS.println("record", c, d)

    inferredRecordLeft, inferredRecordRight = Pair(10, "infer")
    OS.println("record inferred", inferredRecordLeft, inferredRecordRight)

    mixedRecordLeft Int, mixedRecordRight = Pair(11, "partial")
    OS.println("record mixed", mixedRecordLeft, mixedRecordRight)

    recordPairLeft, recordPairRight = Pair(22, "record-pair")
    OS.println("record pair", recordPairLeft, recordPairRight)

    _, skippedOnlyRecordValue = Pair(23, "visible")
    OS.println("record skip only", skippedOnlyRecordValue)

    skippedRecordLeft Int, _, skippedRecordRight Str = Triple(15, "drop", "kept")
    OS.println("record skip", skippedRecordLeft, skippedRecordRight)

    0
}
