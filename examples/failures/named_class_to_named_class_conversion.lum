# FAIL_REGEX:
# no_matching_overload at .*: no constructor overload for class 'Target' matches 1 arguments

class Source {
    name Str
    age Int
}

class Target {
    name Str
    age Int
}

def main() Unit {
    source Source = Source(record {
        name = "Ada"
        age = 10
    })
    _ Target = Target(source)
}
