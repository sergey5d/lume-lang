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
- `impl single Name { ... }` attaches behavior to an explicit `single Name { ... }` declaration
- constructors are dedicated `new { ... } { body }` declarations, where `new { ... }` is the constructor input shape
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

Declaration-body methods are also supported for classes, enums, and singles.
Prefer `impl` for larger types when it improves scanability; keep declaration
body methods available for small types and method-only singles.

Tradeoffs:

- understanding a small type may require reading two blocks
- examples are a little longer than classes with methods in the declaration body
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

- `new { ... }` declares an explicit constructor input shape
- `Type { field: value }` matches explicit constructor input fields by name
- `Type(...)` fills explicit constructor input fields by declaration order
- if any explicit `new` exists, implicit field constructors are suppressed
- if a class has no explicit `new`, field construction uses a synthesized shape from visible fields
- `hidden new { ... } { ... }` hides a constructor from outside callers

## Open Questions

- should multiple `impl A { ... }` blocks be allowed?
- can `impl A` live in another file or module?
- should same-named `single` declarations ever get privileged factory access to class internals?
- should interface conformance remain explicit in `class A with X`?
