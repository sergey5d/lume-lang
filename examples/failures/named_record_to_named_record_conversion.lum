# FAIL_REGEX:
# no_matching_overload at .*: no constructor overload for class 'Profile' matches 1 arguments

record User {
    name Str
    age Int
}

record Profile {
    name Str
    age Int
}

def main() Unit {
    user User = User(record {
        name = "Ada"
        age = 10
    })
    _ Profile = Profile(user)
}
