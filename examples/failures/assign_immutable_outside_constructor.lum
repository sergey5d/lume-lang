# FAIL_REGEX:
# assign_immutable at .*: cannot assign to immutable field 'count' outside constructor

class Counter {
    hidden count Int
}

impl Counter {
    def init(count Int) {
        this.count = count
    }

    def bump() Int {
        this.count = this.count + 1
        this.count
    }
}

def main() Int {
    counter Counter = Counter(1)
    counter.bump()
}
