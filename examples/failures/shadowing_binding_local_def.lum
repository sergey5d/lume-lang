# FAIL_REGEX:
# .*binding 'value' shadows an existing variable; use a different name.*

def main() Int {
    value = 1

    def helper(value Int) Int {
        value + 1
    }

    helper(2)
}
