# EXPECT:
# true
# false
# 0

def canVisitAllRooms(rooms List[List[Int]]) Bool {

    unwrap startKeys <- rooms.get(0) else true

    seen = Set(0)
    keys List[Int] = List()

    for key <- startKeys {
        keys.append(key)
    }

    while keys.size() != 0 {

        key = keys.remove(0).getOr(-1)

        if !seen.contains(key) {
            unwrap roomKeys <- rooms.get(key) else false

            for roomKey <- roomKeys {
                keys.append(roomKey)
            }

            seen.add(key)
        }
    }

    return seen.size() == rooms.size()
}

def main() Int {
    emptyRoom List[Int] = List()

    reachable List[List[Int]] = List(
        List(1),
        List(2),
        List(3),
        emptyRoom
    )

    blocked List[List[Int]] = List(
        List(1, 3),
        List(3, 0, 1),
        List(2),
        List(0)
    )

    OS.println(canVisitAllRooms(reachable))
    OS.println(canVisitAllRooms(blocked))
    0
}
