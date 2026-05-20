# FAIL_REGEX:
# .*binding 'item' shadows an existing variable; use a different name.*

def main() Int {
    item = 1
    unwrap item <- Some(2) else return item
    item
}
