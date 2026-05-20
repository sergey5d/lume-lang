# EXPECT:
# record Ada 10
# public Ben 12 NYC
# vars 3 4
# full Nora 20 Paris
# mixed Liam 8 5
# mixed2 Liam 8 5
# mixed3 Liam 8 5

record User {
    name Str
    age Int
}

class Person {
    name Str
    age Int
    city Str = "NYC"
}

class MutablePoint {
    var x Int
    var y Int
}

class SecretBadge {
    name Str
    hidden code Int
}

impl SecretBadge {
    def init(name Str) {
        this.name = name
        this.code = 99
    }

    def codeValue() Int = code
}

class MixedProfile {
    name Str
    age Int
    hidden score Int = 5
}

impl MixedProfile {
    def scoreValue() Int = score
}

def main() Unit {
    userRecord = record {
        name = "Ada"
        age = 10
    }
    user User = User(userRecord)

    person Person = Person(record("Ben", 12))

    pointRecord = record {
        x = 3
        y = 4
    }
    point MutablePoint = MutablePoint(pointRecord)

    fullPerson Person = Person(record {
        name = "Nora"
        age = 20
        city = "Paris"
    })

    profile MixedProfile = MixedProfile(record {
        name = "Liam"
        age = 8
    })

    profile2 MixedProfile = MixedProfile({
        name = "Liam"
        age = 8
    })

    profile3 MixedProfile = MixedProfile {
        name = "Liam"
        age = 8
    }

    OS.println("record", user.name, user.age)
    OS.println("public", person.name, person.age, person.city)
    OS.println("vars", point.x, point.y)
    OS.println("full", fullPerson.name, fullPerson.age, fullPerson.city)
    OS.println("mixed", profile.name, profile.age, profile.scoreValue())
    OS.println("mixed2", profile2.name, profile2.age, profile2.scoreValue())
    OS.println("mixed3", profile3.name, profile3.age, profile3.scoreValue())
}
