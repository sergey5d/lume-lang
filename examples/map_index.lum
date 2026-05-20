# EXPECT:
# present 1
# missing true
# plusOne 2
# plusOneMissing true

def plusOne(entries Map[Str, Int], key Str) Option[Int] {
    unwrap value <- entries[key]
    Some(value + 1)
}

def main() Unit {
    entries = Map("a": 1, "b": 2)
    present = entries["a"]
    missing = entries["z"]
    plusOneValue = plusOne(entries, "a")
    plusOneMissing = plusOne(entries, "z")

    OS.println("present ${present.getOr(0)}")
    OS.println("missing ${missing.isEmpty()}")
    OS.println("plusOne ${plusOneValue.getOr(0)}")
    OS.println("plusOneMissing ${plusOneMissing.isEmpty()}")
}
