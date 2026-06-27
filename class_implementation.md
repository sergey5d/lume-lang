# Class And Shape Notes

This note captures the current class / shape direction and the tradeoffs around
keeping data declarations separate from behavior.

## Current Shape

The documented style is:

```txt
class A with SomeInterface {
    age Int
    name Str
    hidden malnutritioned Bool = false
}

impl A {
    new {
        age Int
        name Str
    } {
        this.age = age
        this.name = name
    }

    def trueAge() Int = this.age
    def trueName() Str = this.name
}
```

Mental model:

- `class` declares storage, fields, and implemented interfaces
- `shape` declares a structural, read-only field view
- `impl Type { ... }` declares constructors and instance behavior for classes and shapes
- `impl single Name { ... }` declares singleton behavior
- constructors are dedicated `new { params } { body }` declarations
- field access inside methods should use `this.field`

## Why Prefer `impl`

For medium and large types, the split keeps the data shape easy to scan:

```txt
class Account {
    id Str
    owner Str
    hidden var balance Int = 0
}
```

and behavior can grow without burying the fields:

```txt
impl Account {
    def deposit(amount Int) Unit {
        this.balance := this.balance + amount
    }

    def currentBalance() Int = this.balance
}
```

Benefits:

- separates structure from behavior
- keeps class declarations quiet and predictable
- gives interfaces, classes, enums, and singles a consistent extension story
- makes large types easier to navigate

Tradeoffs:

- understanding a small type may require reading two blocks
- examples are a little longer than inline-method classes
- the language needs clear rules for hidden-field access from `impl`

## Constructors

Constructors use dedicated `new` blocks:

```txt
class Person {
    name Str
    age Int
}

impl Person {
    new {
        name Str
    } {
        this.name = name
        this.age = 0
    }
}
```

Current constructor rules:

- if a class has no explicit `new`, eligible field construction uses `Type { field: value }` and `Type(value)`
- if any explicit `new` exists, implicit field constructors are suppressed
- `Type(...)` is positional construction through the available `new` / field construction shape
- `Type { field: value }` is named construction through the available `new` / field construction shape
- `hidden new { ... } { ... }` hides a constructor from outside callers

## Open Questions

- should inline methods remain supported as a convenience, or should `impl` be the only documented behavior home?
- should multiple `impl A { ... }` blocks be allowed?
- can `impl A` live in another file or module?
- should same-named `single` declarations ever get privileged factory access to class internals?
- should interface conformance remain explicit in `class A with X`?
