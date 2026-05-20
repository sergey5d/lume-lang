# SKIP
#
# Candidate class shape where storage stays in `class` and behavior moves into
# a separate `impl` block.

hidden class Person with Named, Aged {

    firstName Str
    lastName Str
    age Int
    city Str

    hidden var archived Bool
    var internalScore Int = 1
}

impl Person {

    def init(firstName Str, lastName Str) {
        this.firstName = firstName
        this.lastName = lastName
        this.age = 18
        this.city = "unknown"
        this.archived = false
    }

    hidden def calc() Int {
        firstName.size() + lastName.size()
    }

    def fullName() Str = firstName + " " + lastName

    def isAdult() Bool = age >= 18

    def moveTo(newCity Str) Unit {
        city := newCity
    }

    def celebrateBirthday() Unit {
        age += 1
        internalScore += 10
    }

    def archive() Unit {
        archived := true
    }

    def debugLabel() Str =
        fullName() + "@" + city + ":" + age + ":" + internalScore + ":" + archived
}
