# FAIL_REGEX:
# no_matching_overload at .*: class/record 'MixedProfile' requires an anonymous record with exactly matching field names and types

class MixedProfile {
    name Str
    age Int
    hidden score Int = 5
}

def main() Unit {
    _ MixedProfile = MixedProfile(record {
        name = "Liam"
        age = 8
        score = 5
    })
}
