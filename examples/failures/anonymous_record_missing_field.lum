# FAIL_REGEX:
# invalid_argument_type at .*: cannot pass \{name Str\} to parameter of type \{name Str, age Int\}

def describe(user { name Str, age Int }) Str =
    user.name

def main() Unit {
    value = record { name = "Ana" }
    OS.println(describe(value))
}
