# Syntax Cleanup Notes

This file captures readability-oriented syntax ideas against the current Lume
surface. It is intentionally secondary to `syntax.md`, which is the reference
for what works today.

## Current Direction

The language is strongest when it keeps one obvious form for each major idea:

- `class` for nominal instance types
- `single` for singleton namespaces/values
- `enum` for tagged sums
- `interface` for contracts
- `impl Type { ... }` and `impl single Name { ... }` for behavior
- `use` for module imports
- `new { params } { body }` for explicit constructors
- `Type { field: value }` for named construction
- `{ field: value }` for anonymous shapes
- `try` for propagation
- `expect` for assertive refutable binding or boolean assertions
- explicit lambdas such as `value -> value + 1`

The main cleanup principle still stands:

- prefer regularity over clever compactness
- keep punctuation meaningful
- avoid adding a second spelling unless it buys real readability

## Settled Cleanup

These older surfaces should stay out of new examples and docs:

- `import`; use `use`
- named `record` declarations; use `shape` for structural data or `class` for nominal types
- anonymous `class { ... }` / `record { ... }`; use plain `{ ... }`
- named fields with `=` inside construction; use `field: value`
- language-level `unwrap` forms; use `let ... else`, `expect`, or `try`
- placeholder expression lambdas like `_ + 1`; use `x -> x + 1`
- class-to-tuple destructuring; use class/anonymous-shape brace destructuring
- `Type({ ... })` nominal conversion; use explicit construction or a future `anon as Type` form if adopted

## Keep

These still feel like strong surface choices:

- `def`
- `var`
- `class`
- `single`
- `enum`
- `interface`
- `public`
- `hidden`
- `match` with mandatory `case`
- `partial` as the partial-match form
- `let`
- `expect`
- `defer`
- `@Annotation(...)`
- `Type { ... }`
- `use module/path`
- `->` for lambdas and function types
- string interpolation
- `with`

Notes:

- `case` makes `match` blocks easier to scan.
- `public` and `hidden` are readable visibility markers.
- `expect` is a better assertive word than reviving `unwrap` syntax.

## Places To Keep Watching

### 1. Method Placement

Current preferred style:

```txt
class Person {
    name Str
    age Int
}

impl Person {
    def label() Str = this.name + " " + this.age
}
```

Open question:

- should inline methods remain supported as a convenience?
- or should `impl` become the only documented home for behavior?

The `impl` split reads better for medium and large types, but tiny examples can
feel a little heavier.

### 2. Single-Line Body Forms

Current direction:

- brace-delimited `if` is preferred
- `for`, `match`, and `partial` are block-only
- same-line `else expr` and `yield expr` are valid
- if a body moves to the next line, use `{ ... }`

This keeps shorthand useful without making newlines do too much hidden work.

### 3. Constructor Surface

Current direction:

Structural construction when no explicit `new` exists:

```txt
class User {
    name Str
    age Int
}

user User = User { name: "Ada", age: 10 }
```

Explicit constructor calls when `new` exists:

```txt
class NamedUser {
    name Str
    age Int
}

impl NamedUser {
    new {
        name Str
    } {
        this.name = name
        this.age = 0
    }
}

user NamedUser = NamedUser("Ada")
```

Open questions:

- should same-named `single` factories get any privileged access?
- do we need more explicit syntax for hiding generated construction paths?
- should anonymous-shape-to-class conversion use `anon as User` later?

### 4. Pattern-Lambda Sugar

Current explicit style:

```txt
values.map(value -> match value {
    case Some(x) => x + 1
    case None => 0
})
```

Possible future shorthand:

```txt
values.map(match {
    case Some(x) => x + 1
    case None => 0
})
```

This should stay future-only until there is a clear readability win. It is
contextual magic, and the explicit lambda is already understandable.

### 5. Symbolic Operators

Operator overloading exists, but it should stay conservative.

Good candidates:

- arithmetic-like value types
- vectors / matrices / geometry values
- domain values such as `Money`, `Distance`, or `Duration`

Risky candidates:

- symbolic forms that hide domain behavior
- aliases for ordinary named methods
- clever collection punctuation when `add`, `addAll`, `from`, or `put` reads better

## Preferred Example Style

```txt
use model/user/User

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

    def label() Str = this.name + " " + this.age
}

def classify(value Option[Int]) Int =
    match value {
        case Some(x) => x
        case None => 0
    }

anon = {
    name: "Ada"
    age: 10
}

person Person = Person { name: anon.name, age: anon.age }

for i <- Range(0, 10) {
    OS.println(i)
}
```

## Summary

The syntax should keep moving toward:

- fewer alternate forms
- less contextual sugar
- explicit behavior homes
- clear construction syntax
- explicit lambdas unless a shorthand is obviously better
