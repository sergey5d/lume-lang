# EXPECT:
# some 6
# none missing
# optSome 8
# optNone true
# pair 5-no-right
# sum 9
# sumFail sum-missing

def plusOne(value Option[Int]) Result[Int, Str] {
    unwrap item <- value else Err("missing")
    Ok(item + 1)
}

def plusThree(value Option[Int]) Option[Int] {
    unwrap item <- value
    Some(item + 3)
}

def pairwise(left Option[Int], right Option[Str]) { count Int, label Str } = {
    unwrap count <- left else {
        record { count = 0, label = "no-left" }
    }
    unwrap label <- right else {
        record { count = count, label = "no-right" }
    }
    record(count, label)
}

def sumBoth(left Option[Int], right Option[Int]) Result[Int, Str] {
    unwrap {
        a <- left
        b <- right
    } else {
        Err("sum-missing")
    }
    Ok(a + b)
}

def main() Unit {
    some = plusOne(Some(5))
    none = plusOne(None())
    optSome = plusThree(Some(5))
    optNone = plusThree(None())
    pair = pairwise(Some(5), None())
    sum = sumBoth(Some(4), Some(5))
    sumFail = sumBoth(Some(4), None())

    OS.println("some ${some.getOr(0)}")
    OS.println("none ${none.getError()}")
    OS.println("optSome ${optSome.getOr(0)}")
    OS.println("optNone ${optNone.isEmpty()}")
    OS.println("pair ${pair.count}-${pair.label}")
    OS.println("sum ${sum.getOr(0)}")
    OS.println("sumFail ${sumFail.getError()}")
}
