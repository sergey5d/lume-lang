# EXPECT:
# option some 6
# option none true
# result ok 7
# result err bad
# either right 8
# either left nope
# either combo 11
# either combo left size bad

def plusOneOption(value Option[Int]) Option[Int] {
    unwrap item <- value
    return Some(item + 1)
}

def plusOneResult(value Result[Int, Str]) Result[Int, Str] {
    unwrap item <- value
    return Ok(item + 1)
}

def plusOneEither(value Either[Str, Int]) Either[Str, Int] {
    unwrap item <- value
    return Right(item + 1)
}

def twoEithers(value Either[Str, Int], value2 Either[Str, Str]) Either[Str, Int] {
    unwrap item <- value
    unwrap str <- value2
    unwrap size <- value2.map((s Str) -> s.size())
    return Right(item + str.size() / 2 + size)
}

def main() {
    optionSome = plusOneOption(Some(5))
    optionNone = plusOneOption(None())

    OS.println("option some ${optionSome.getOr(0)}")
    OS.println("option none ${optionNone.isEmpty()}")

    resultOk = plusOneResult(Ok(6))
    resultErr = plusOneResult(Err("bad"))

    OS.println("result ok ${resultOk.getOr(0)}")
    OS.println("result err ${resultErr.getError()}")

    eitherRight = plusOneEither(Right(7))
    eitherLeft = plusOneEither(Left("nope"))
    OS.println("either right ${eitherRight.getOr(0)}")
    OS.println("either left ${eitherLeft.expectLeft()}")

    if true {
        combo = twoEithers(Right(7), Right("abc"))
        comboLeft = twoEithers(Right(7), Left("size bad"))
        OS.println("either combo ${combo.getOr(0)}")
        OS.println("either combo left ${comboLeft.expectLeft()}")
    }
}
