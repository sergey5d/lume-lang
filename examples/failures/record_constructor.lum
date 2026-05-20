# FAIL_REGEX:
# invalid_record_method at .*: record 'Bad': records cannot declare constructors

record Bad {
    value Int
}

impl Bad {
    def init(value Int) {
        init(value = value)
    }
}

def main() Int = 0
