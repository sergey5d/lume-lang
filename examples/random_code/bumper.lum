package bumper

class Counter {
    hidden var count Int
}

impl Counter {
    def init(count Int) {
        this.count = count
    }

    def bump(delta Int) Int {
        this.count += delta
        return this.count
    }
}

seed Int = 1

def run() Int {
    c Counter = Counter(seed)
    return c.bump(2)
}