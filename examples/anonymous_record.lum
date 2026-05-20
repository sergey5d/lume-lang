# EXPECT:
# Ana is 10
# Ben is 12
# 1
# 20
# Ada-10
# Cara is 14
# 11
# Ben 12
# Ana 10
# 6

def describe(user { name Str, age Int }) Str =
    user.name + " is " + user.age

def makeCounter(base Int) { count Int, next Int } = {
    return record(base, base + 1)
}

def main() Unit {
    full = record {
        name = "Ana"
        age = 10
        city = "NYC"
    }
    smaller = record {
        name = "Ben"
        age = 12
    }
    a = 1
    inferred = record {
        count = a
    }
    mixed = record { a = 5, c = 7,
        b = 8
    }
    mixedShape = record { name = "Ada",
        age = 10
    }
    narrow { name Str, age Int } = full
    positional { name Str, age Int } = record("Ben", 12)

    OS.println(describe(full))
    OS.println(describe(smaller))
    OS.println(inferred.count)
    OS.println(mixed.a + mixed.b + mixed.c)
    OS.println(mixedShape.name + "-" + mixedShape.age)
    OS.println(describe(record("Cara", 14)))
    counter = makeCounter(5)
    OS.println(counter.count + counter.next)
    OS.println(positional.name + " " + positional.age)
    OS.println(narrow.name + " " + narrow.age)
    typedCounter { count Int, next Int } = makeCounter(5)
    OS.println(typedCounter.next)
}
