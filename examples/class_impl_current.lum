# SKIP
#
# Legacy class shape from before the class/record grammar switch, kept only as
# a comparison artifact.

class Person with Named, Aged {
    firstName Str
    lastName Str
    age Int
    city Str

    hidden var archived Bool = false
    hidden var internalScore Int = 0

    def init(fullName Str, age Int, city Str) {
        parts = fullName.split(" ")
        this.firstName = parts.get(0).getOr("")
        this.lastName = parts.get(1).getOr("")
        this.age = age
        this.city = city
    }

    def init(firstName Str, lastName Str) {
        this.firstName = firstName
        this.lastName = lastName
        this.age = 18
        this.city = "unknown"
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
