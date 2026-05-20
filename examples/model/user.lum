# EXPECT:
# Ada
# 36

package user

class Person {
    name Str
    age Int
}

impl Person {
    def name() Str = name

    def age() Int = age
}

def main() Unit {
    person = Person(name = "Ada", age = 36)
    OS.println(person.name())
    OS.println(person.age())
}
