# FAIL_REGEX:
# no_matching_overload at .*: class/record 'SecretBadge' cannot be built from an anonymous record because it has private fields without initializers

class SecretBadge {
    name Str
    hidden code Int
}

impl SecretBadge {
    def init(name Str) {
        this.name = name
        this.code = 99
    }
}

def main() Unit {
    _ SecretBadge = SecretBadge(record {
        name = "Mia"
    })
}
