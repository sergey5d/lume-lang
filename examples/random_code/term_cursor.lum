# EXPECT:
# valid0 true
# loc0 0:0
# valid1 true
# loc1 0:2
# valid2 true
# loc2 0:2
# valid3 true
# loc3 1:1
# valid4 false
# loc4 -1:-1
# to be or not to be:
# document: 0
# document: 1
# to be or not to be v2:
# document: 0
# document: 1
# document: 5
# document: 7
# to be or not to be v3:
# document: 0
# document: 1
# document: 5
# document: 7
# 0

record Location {
    docId Int
    position Int
}

class Cursor {
    hidden term Str
    hidden documents List[Str]
    hidden var current Location
    hidden var valid Bool = false
}

impl Cursor {
    def init(term Str, documents List[Str]) {
        this.term = term
        this.documents = documents
        this.current := Location(-1, -1)
        this.valid := false
        this.seek(Location(-1, -1))
    }

    def isValid() Bool = valid

    def get() Location = current

    def advance() Unit {
        if valid {
            this.seek(Location(current.docId, current.position + 1))
        }
    }

    def seek(location Location) Unit {
        var docId Int = 0
        var found Bool = false

        if location.docId >= 0 {
            docId := location.docId
        }

        while docId < documents.size() && !found {
            docTerms = documents[docId].split(" ")
            var pos Int = 0

            if docId == location.docId && location.position >= 0 {
                pos := location.position
            }

            while pos < docTerms.size() && !found {
                if docTerms[pos] == term {
                    this.current := Location(docId, pos)
                    this.valid := true
                    found := true
                }
                if !found {
                    pos += 1
                }
            }

            if !found {
                docId += 1
            }
        }

        if !found {
            this.current := Location(-1, -1)
            this.valid := false
        }
    }
}

def toBeOrNotToBe() Unit {

    documents List[Str] = [
        "this is to be",
        "method to to be be haha",
        "this is to cat",
        "method to to be haha",
        "things can't be to"
    ]

    OS.println("to be or not to be:")

    c1 = Cursor("to", documents)
    c2 = Cursor("be", documents)
    
    var loc1 = c1.get()
    var loc2 = c2.get()

    result List[Int] = []

    while c1.isValid() && c2.isValid() {

        loc1 := c1.get()
        loc2 := c2.get()

        if loc1.docId > loc2.docId {
            c2.seek(Location(loc1.docId, 0))
        } else if loc1.docId < loc2.docId {
            c1.seek(Location(loc2.docId, 0))
        } else {

            currDocId = loc1.docId
            c2Stack List[Int] = []

            while c2.isValid() && loc2.docId == currDocId {
                c2Stack.append(loc2.position)
                c2.advance()
                loc2 := c2.get()
            }

            c1Orig = loc1

            var failed = false

            while c1.isValid() && loc1.docId == currDocId && !c2Stack.isEmpty() {
                unwrap c2Loc <- c2Stack.removeLast() else OS.panic()
                if c2Loc < loc1.position {
                    failed := true
                    break
                }
                c1.advance()
                loc1 := c1.get()
            }

            if !failed && c2Stack.isEmpty() && loc1.docId > c1Orig.docId {
                result.append(c1Orig.docId)
            } else {
                c1.seek(Location(loc2.docId, 0))
            }
        }
    }

    for res <- result {
        OS.println("document: " + res)
    }
}

def toBeOrNotToBe2() Unit {

    documents List[Str] = List(
        "this is to be",
        "method to to be be haha",
        "this is to cat",
        "method to to be haha",
        "things can't be to",
        "to equasion has to be",
        "to equasion is not be",
        "to second equasion has to be"
    )

    OS.println("to be or not to be v2:")

    c1 = Cursor("to", documents)
    c2 = Cursor("be", documents)
    
    def advanceContiniously(c Cursor) { begin Int, count Int, docId Int } = {
        var counter = 1
        var loc = c.get()
        begin = loc.position
        docId = loc.docId
        var prev = loc.position

        c.advance()
        loc := c.get()

         while c.isValid() && prev + 1 == loc.position && docId == loc.docId {
            counter += 1
            prev := loc.position
            c.advance()
            loc := c.get()
        }
        record(begin, counter, docId)
    }

    def alignDocs(doc1 Int, doc2 Int) {
        if doc1 > doc2 {
            c2.seek(Location(doc1, 0))
        } else if doc1 < doc2 {
            c1.seek(Location(doc2, 0))
        }
    }

    result List[Int] = []

    var advance1 = advanceContiniously(c1)
    var advance2 = advanceContiniously(c2)

    alignDocs(advance1.docId, advance2.docId)

    while c1.isValid() || c2.isValid() {

        if advance1.docId > advance2.docId {
            alignDocs(advance1.docId, advance2.docId)
            advance2 := advanceContiniously(c2)
        } else if advance1.docId < advance2.docId {
            alignDocs(advance1.docId, advance2.docId)
            advance1 := advanceContiniously(c1)
        } else {
            if advance1.begin + advance1.count == advance2.begin {
                if advance1.count == advance2.count {
                    docId = advance1.docId

                    advance1 := advanceContiniously(c1)
                    advance2 := advanceContiniously(c2)

                    if (advance1.docId > docId ||  !c1.isValid()) && 
                            (advance2.docId > docId || !c2.isValid()) {
                        result.append(docId)
                    }
                } else {
                    advance1 := advanceContiniously(c1)
                    advance2 := advanceContiniously(c2)
                }
            } else {
                if advance1.begin > advance2.begin {
                    advance2 := advanceContiniously(c2)
                } else {
                    advance1 := advanceContiniously(c1)
                }
            }
        }
    }

    if advance1.docId == advance2.docId && 
            advance1.begin + advance1.count == advance2.begin && advance1.count == advance2.count {
        result.append(advance1.docId)
    }

    for res <- result {
        OS.println("document: " + res)
    }
}

def toBeOrNotToBe3() Unit {

    documents List[Str] = List(
        "this is to be",
        "method to to be be haha",
        "this is to cat",
        "method to to be haha",
        "things can't be to",
        "to equasion has to be",
        "to equasion is not be",
        "to second equasion has to be"
    )

    OS.println("to be or not to be v3:")

    toCursor = Cursor("to", documents)
    beCursor = Cursor("be", documents)

    def collectPositions(cursor Cursor, docId Int) List[Int] {
        positions List[Int] = []
        while cursor.isValid() && cursor.get().docId == docId {
            positions.append(cursor.get().position)
            cursor.advance()
        }
        positions
    }

    def hasBeRun(bePositions List[Int], begin Int, count Int) Bool {
        var startIndex Int = 0
        var found = false

        while startIndex < bePositions.size() && !found {
            if bePositions[startIndex] == begin {
                found := true
            } else {
                startIndex += 1
            }
        }

        if !found {
            return false
        }

        var offset Int = 0
        while offset < count {
            currentIndex = startIndex + offset
            if currentIndex >= bePositions.size() || bePositions[currentIndex] != begin + offset {
                return false
            }
            offset += 1
        }

        true
    }

    def containsPattern(toPositions List[Int], bePositions List[Int]) Bool {
        var toStart Int = 0

        while toStart < toPositions.size() {
            startPos = toPositions[toStart]
            if !(toStart > 0 && toPositions[toStart - 1] == startPos - 1) {
                var runLen Int = 0

                while toStart + runLen < toPositions.size() && toPositions[toStart + runLen] == startPos + runLen {
                    runLen += 1
                }

                var count Int = 1
                while count <= runLen {
                    if hasBeRun(bePositions, startPos + count, count) {
                        return true
                    }
                    count += 1
                }
            }

            toStart += 1
        }

        false
    }

    result List[Int] = []

    while toCursor.isValid() || beCursor.isValid() {
        var docId Int = -1

        if toCursor.isValid() && beCursor.isValid() {
            if toCursor.get().docId < beCursor.get().docId {
                docId := toCursor.get().docId
            } else {
                docId := beCursor.get().docId
            }
        } else if toCursor.isValid() {
            docId := toCursor.get().docId
        } else {
            docId := beCursor.get().docId
        }

        var toPositions List[Int] = []
        if toCursor.isValid() && toCursor.get().docId == docId {
            toPositions := collectPositions(toCursor, docId)
        }

        var bePositions List[Int] = []
        if beCursor.isValid() && beCursor.get().docId == docId {
            bePositions := collectPositions(beCursor, docId)
        }

        if containsPattern(toPositions, bePositions) {
            result.append(docId)
        }
    }

    for res <- result {
        OS.println("document: " + res)
    }
}

def main() Int {
    documents List[Str] = List(
        "cat dog cat",
        "bird cat",
        "dog"
    )

    cursor = Cursor("cat", documents)

    OS.println("valid0 " + cursor.isValid())
    loc0 = cursor.get()
    OS.println("loc0 " + loc0.docId + ":" + loc0.position)

    cursor.advance()
    OS.println("valid1 " + cursor.isValid())
    loc1 = cursor.get()
    OS.println("loc1 " + loc1.docId + ":" + loc1.position)

    cursor.seek(loc1)
    OS.println("valid2 " + cursor.isValid())
    loc2 = cursor.get()
    OS.println("loc2 " + loc2.docId + ":" + loc2.position)

    cursor.advance()
    OS.println("valid3 " + cursor.isValid())
    loc3 = cursor.get()
    OS.println("loc3 " + loc3.docId + ":" + loc3.position)

    cursor.advance()
    OS.println("valid4 " + cursor.isValid())
    loc4 = cursor.get()
    OS.println("loc4 " + loc4.docId + ":" + loc4.position)

    toBeOrNotToBe()
    toBeOrNotToBe2()
    toBeOrNotToBe3()

    0
}
