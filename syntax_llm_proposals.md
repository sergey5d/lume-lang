# Syntax Proposals

This file captures a readability-first review of the current language syntax.

Goals:

- readable
- as little noise as possible

Main principle:

- prefer one obvious way to write a thing over multiple short-but-clever forms

## Overall Take

The language already has a readable core. The main thing hurting clarity is not punctuation by itself, but the number of alternate forms and context-sensitive shortcuts.

The best direction seems to be:

- keep the explicit keyword-based core
- trim magic/contextual sugar
- reduce the number of ways to express the same construct
- favor regularity over ultra-shortness

## Keep

These seem like strong parts of the current surface syntax and should probably stay:

- `def`
- `var`
- `class`
- `record`
- `object`
- `enum`
- `interface`
- `public`
- `hidden`
- `match` with mandatory `case`
- `unwrap`
- `@Annotation(...)`
- `record { ... }`
- path-style imports
- `->` for lambdas and function types
- string interpolation
- `with`

Notes:

- `case` improves readability more than it adds noise
- `record { ... }` is explicit and much clearer than trying to reuse plain `{ ... }`
- `public` / `hidden` are honest, readable visibility markers

## Trim First

These are the first places where the syntax seems too clever relative to the value it gives.

### 2. Remove tuple-range magic in `for`

Current issue:

- `(0, 10)` means tuple almost everywhere
- but inside `for` it can mean range
- that makes tuples carry two meanings

Preferred direction:

- use `Range(0, 10)` now
- maybe introduce a dedicated range form later if truly needed

Preferred style:

```txt
for i <- Range(0, 10) {
    println(i)
}
```

### 3. Remove positional anonymous record construction

Current issue:

- `record("Ada", 10)` depends on contextual shape
- it is short, but it adds hidden rules
- it makes anonymous record construction less obvious

Preferred direction:

- keep only named-field anonymous record construction

Preferred style:

```txt
user = record {
    name = "Ada"
    age = 10
}
```

### 4. Trim operator overloading hard

Current issue:

- many symbolic operators increase surface area quickly
- they tend to reduce readability more than they reduce noise

Preferred direction:

- keep arithmetic if needed
- keep indexing if needed
- strongly reconsider symbolic custom forms like:
  - `:+`
  - `:-`
  - `++`
  - `--`
  - `|`
  - `&`
  - `<<`
  - `>>`
  - `::`

The language reads better when behavior is mostly in methods and keywords rather than symbolic cleverness.

### 5. Reconsider wildcard imports long-term

Current issue:

- `import pkg/*` is short
- but it becomes harder to read where names came from

Preferred direction:

- prefer:
  - `import pkg/sub`
  - `import pkg/sub/{A, B as C}`

- use wildcard import sparingly, or phase it down later

## Simplify

These are not necessarily bad, but they would benefit from choosing one main form.

### 1. Unify method placement

Current issue:

- classes and records use `impl`
- objects and enums can define methods inline
- that creates an avoidable split in the mental model

Preferred direction:

- choose one model

Most readable option:

- allow methods inline everywhere
- make `impl` optional or remove it later

That would reduce grammar branching and make declaration reading simpler.

### 2. Unify `if`

Current issue:

- statement `if`
- block expression `if`
- shorthand `if ... then ... else ...`

Preferred direction:

- keep one expression story:

```txt
result = if cond {
    1
} else {
    0
}
```

- drop `then`

This is slightly longer, but much simpler overall.

### 3. Keep `case` mandatory in `match` and `partial`

This was a good change and should stay.

Preferred style:

```txt
match value {
    case Some(x) => x
    case None => 0
}
```

```txt
partial value {
    case Some(x) => x
}
```

### 4. Keep `unwrap`, but avoid too many more binding variants

Current syntax already has good coverage:

- `if x <- maybe`
- `unwrap x <- maybe`
- block `unwrap { ... }`

That is already a strong feature set. Additional special binding syntaxes should justify themselves carefully.

## Areas That Feel Too Clever

These features reduce characters but increase hidden rules:

- contextual `match { ... }` lambda sugar
- contextual `partial { ... }` lambda sugar
- placeholder lambdas like `_ + 1`
- shape-driven positional `record(...)`
- tuple-as-range in `for`

These should be treated with caution.

The risk is not that they are impossible to learn. The risk is that they make the language feel less regular.

## Where Extra Keywords Are Worth It

These keywords are good noise:

- `case`
- `public`
- `hidden`
- `unwrap`
- `record`

They make programs more explicit and reduce ambiguity.

## Strongest 5 Cleanup Proposals

If only five changes are made, the best candidates seem to be:

1. remove `apply` as a language calling rule
2. remove tuple-range magic from `for`
3. remove positional anonymous `record(...)`
4. pick one `if` expression form and remove `then`
5. unify method placement across all type declarations

## Preferred Style Direction

Example of the kind of syntax style that seems strongest:

```txt
public record Person {
    age Int
    name Str

    def init(age Int, name Str) {
        this.age = age
        this.name = name
    }

    def label() Str = name
}

def classify(value Option[Int]) Int =
    match value {
        case Some(x) => x
        case None => 0
    }

user = record {
    name = "Ada"
    age = 10
}

for i <- Range(0, 10) {
    println(i)
}

result = if count > 0 {
    "ok"
} else {
    "empty"
}
```

## Summary

The language should probably move toward:

- fewer alternate forms
- fewer context-sensitive shortcuts
- fewer symbolic tricks
- more regular declaration and control-flow syntax

In short:

- keep the explicit keyword-based core
- cut magic
- prefer regularity over clever compactness

## Additional Notes

These are active design notes worth considering separately from the earlier trim/simplify recommendations.

### Consider replacing `class` / `record` with a `type` modifier

Possible direction:

- unify declaration vocabulary around `type`
- then use modifiers or secondary markers to express the shape/semantics

Examples:

```txt
type Person {
    age Int
    name Str
}
```

or:

```txt
type record Person {
    age Int
    name Str
}
```

Potential benefit:

- fewer top-level declaration keywords
- more uniform surface area

Potential risk:

- `class`, `record`, `enum`, `object`, `interface` currently communicate meaning very directly
- collapsing them under `type` may reduce immediate readability unless the replacement is extremely clear

### Consider a lighter pattern-first `match` expression form

Possible direction:

```txt
match maybeUser {
    some user -> return user.name
    none -> return "Unknown"
}
```

Potential benefit:

- more concise than `case Some(user) => ...`
- may read more naturally for common `Option`-style flows

Potential risk:

- introduces a second match surface alongside the general `case ... => ...` form
- can make `Option`/`Result`-style matching feel special instead of uniform
- `some` / `none` would need a clear relationship to enum cases and ordinary constructor patterns

### Consider `try` instead of `unwrap`

Possible direction:

```txt
fn loadUser(id: UserId) async -> Result<User, UserError> {
    let response = try await http.get("/users/" + id)
    return try parseUser(response.body)
}
```

Potential benefit:

- very familiar to many users
- reads naturally for propagation-oriented code
- may compose better than statement-shaped `unwrap` in expression-heavy flows

Potential risk:

- `try` usually implies one specific propagation protocol, while current `unwrap` is a more explicit language form
- if both `try` and `unwrap` exist, the language may end up with two parallel unwrapping idioms
- adopting `try` well may push the language toward a more expression-oriented error-handling model overall

Open question:

- whether `try` should replace `unwrap`
- or whether `try` should become only a shorthand for propagation while `unwrap` remains the more explicit binding-oriented form
