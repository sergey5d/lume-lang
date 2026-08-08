# Function Types

Lume uses explicit `fn(...) => T` syntax for function types while keeping named
functions and lambda expressions keyword-free. `syntax.md` remains the primary
language reference; this document explains the rationale and migration.

## Motivation

Requiring `def` on nearly every function merely to distinguish the relatively
rare function-valued binding puts the readability cost on the common case.
Instead, mark the uncommon type:

```txt
getCurrentTime() Int =
    0

getTimeRef fn() => Int =
    getCurrentTime

adderRef fn(Int, Int) => Int =
    (left, right) => left + right
```

These forms remain visually distinct:

```txt
calculate(value Int) Int = ...  # named function declaration
mapper fn(Int) => Str = ...     # binding containing a function value
value => value.toStr()          # anonymous function expression
```

## Syntax

Use `fn(...) => T` as the canonical function-type syntax in every type
position. Write `fn` directly before the parameter list, without a space.

```txt
run(operation fn() => Unit) Unit =
    operation()

mapValue(value Int, mapper fn(Int) => Str) Str =
    mapper(value)

handlers [Str : fn(Request) => Response]
callbacks [fn(Event) => Unit]
```

The marker describes a function value, not how it was created. A value of a
function type may be a lambda, top-level function, bound instance method, or
named-object method:

```txt
getCurrentTime
clock.currentTime
Service.load
value => value.toStr()
```

`lambda(...) => T` would describe only one possible source of the value.
`type[(...) => T]` is more verbose, resembles `Type[T]` runtime metadata, and
becomes difficult to read when nested.

## By-Name Parameters

This syntax makes the distinction between a by-name expression and a thunk
explicit:

```txt
fallback => Int       # delayed expression, evaluated when read
fallback fn() => Int  # zero-argument function value
```

For example:

```txt
twice(value => Int) Int =
    value + value

twiceThunk(value fn() => Int) Int =
    value() + value()

twice(expensive())
twiceThunk(() => expensive())
```

Use by-name parameters for conditional-value APIs. Use `fn() => T` when a
caller passes, stores, returns, or explicitly invokes the work itself.

## Higher-Order Types

The `fn` marker keeps nested function types readable:

```txt
factory fn() => fn(Int) => Str
transform fn(Int) => fn(Str) => Bool

compose(
    first fn(Int) => Str,
    second fn(Str) => Bool
) fn(Int) => Bool =
    value => second(first(value))
```

## Declaration Disambiguation

Named function declaration parameters should have explicit types. Lambda
parameters may continue to infer their types.

```txt
sum(left Int, right Int) Int =
    left + right

pair (Int, Int) =
    (1, 2)
```

This lets the parser distinguish a named function declaration from a
tuple-typed binding by their internal grammar rather than whitespace around
`(`. Zero-argument forms remain unambiguous:

```txt
now() Int = 0
nowRef fn() => Int = now
```

The language should not rely on a `name(...)` versus `name (...)` distinction.

## Migration

Do not retain two permanent function-type spellings. Replace:

```txt
(Int) => Str
```

with:

```txt
fn(Int) => Str
```

Lambda expressions and match branches continue to use `=>` without `fn`:

```txt
mapper fn(Int) => Str =
    value => value.toStr()

result = match option {
    case Some(value) => value.toStr()
    case None => "missing"
}
```

Here `=>` retains its broad "produces" meaning, while `fn` marks a function
type.

## Recommended Model

```txt
# Named function
getCurrentTime() Int =
    0

# Function-valued binding
getTimeRef fn() => Int =
    getCurrentTime

# Function-valued parameter
run(operation fn() => Unit) Unit =
    operation()

# Function-valued return
makeAdder(base Int) fn(Int) => Int =
    value => base + value

# By-name parameter
getOr(defaultValue => Int) Int =
    ...

# Lambda expression
increment =
    value => value + 1
```

The language model is therefore:

- keep named functions keyword-free
- require `fn(...) => T` for function types
- keep arrow-only lambda expressions
- place the explicit marker on the uncommon function-value type rather than
  on every named function declaration
