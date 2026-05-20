# EXPECT:
# 7
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

class PairBox {
    left Int
    right Int
}

def main() Int {
    workerLike WorkerLike = Worker()
    OS.println(match workerLike {
       case worker Worker => worker.doWork()
       case _ Other => 100
       case _ => 0
    })

    pair PairBox = PairBox(4, 9)
    result = match pair {
       case PairBox(left, right) => left + right
    }
    if result != 13 {
        return 1
    }

    0
}
