# FAIL_REGEX:
# invalid_record_update at .*: class update requires a class without private fields

class SecretAmount {
    amount Int
    hidden secret Str = "hidden"
}

def main() Unit {
    value = SecretAmount(10)
    updated = value with { amount = 42 }
}
