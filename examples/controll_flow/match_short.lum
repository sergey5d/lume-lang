# EXPECT:
# pair 5-9
# 14
# worker 7
# 0

class PairBox {
    left Int
    right Int
}

interface WorkerLike {
    def doWork() Int
}

class Worker with WorkerLike {
}

impl Worker {
    def doWork() Int = 7
}

def main() Int {
    pair PairBox = PairBox(5, 9)

    match pair {
        case PairBox(left, right) => OS.println("pair " + left + "-" + right)
    }

    picked = match pair {
        case PairBox(left, right) => left + right
    }
    OS.println(picked)

    workerLike WorkerLike = Worker()
    match workerLike {
        case worker Worker => OS.println("worker " + worker.doWork())
    }

    0
}
