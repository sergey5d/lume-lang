# EXPECT:
# worker 7
# other 3
# slacker -1
# 0

interface WorkerLike {
    def doWork() Int
}

class Worker with WorkerLike {
}

impl Worker {
    def doWork() Int = 7
}

class Other with WorkerLike {
}

impl Other {
    def doWork() Int = 3
}

class Slacker with WorkerLike {
}

impl Slacker {
    def doWork() Int = -1
}

def describe(value WorkerLike) {
    match value {
        case worker Worker => {
            OS.println("worker " + worker.doWork())
        }
        case other Other => {
            OS.println("other " + other.doWork())
        }
        case _ Slacker =>
            OS.println("slacker " + value.doWork())
        case _ => {
            OS.println("unknown")
        }
    }
}

def main() Int {
    describe(Worker())
    describe(Other())
    describe(Slacker())
    0
}
