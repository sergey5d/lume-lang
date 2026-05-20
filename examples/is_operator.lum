# EXPECT:
# value is Counter? true
# counter is CounterPrecursor? true
# string is Str? true
# value is Str? false
# counter is Breaker? false
# 0

interface CounterPrecursor {
}

class Counter with CounterPrecursor {
}

interface Breaker {
}

def main() Int {
	counter = Counter()
	OS.println("value is Counter? " + (counter is Counter))
	OS.println("counter is CounterPrecursor? " + (counter is CounterPrecursor))
	OS.println("string is Str? " + ("hello" is Str))
	OS.println("value is Str? " + (counter is Str))
	OS.println("counter is Breaker? " + (counter is Breaker))
	0
}
