# Function Types

Lume uses `def` for named function and method declarations and explicit
`fn(...) T` syntax for function types. Lambda expressions remain
keyword-free. `syntax.md` remains the primary language reference; this document
explains how those forms fit together.

## Motivation

`def` makes declaration sites explicit, while `fn` makes function-valued types
explicit. The two markers distinguish declarations, stored callable values,
and lambdas without relying on whitespace or type-directed parsing:

```txt
def getCurrentTime() Int =
    0

getTimeRef fn() Int =
    getCurrentTime

adderRef fn(Int, Int) Int =
    (left, right) => left + right
```

These forms remain visually distinct:

```txt
def calculate(value Int) Int = ...  # named function declaration
mapper fn(Int) Str = ...     # binding containing a function value
value => value.toStr()          # anonymous function expression
```

## Syntax

Use `fn(...) T` as the canonical function-type syntax in every type position.
Whitespace between `fn` and the parameter list is insignificant, so
`fn (Int) Str` is equivalent to `fn(Int) Str`. Write the return type after the
parameter list without an arrow.

```txt
def run(operation fn() Unit) Unit =
    operation()

def mapValue(value Int, mapper fn(Int) Str) Str =
    mapper(value)

handlers [Str : fn(Request) Response]
callbacks [fn(Event) Unit]
```

The canonical compact spelling is preferred in formatted code, but both forms
are valid:

```txt
mapper fn(Int) Str
mapper fn (Int) Str
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
fallback fn() Int  # zero-argument function value
```

For example:

```txt
def twice(value => Int) Int =
    value + value

def twiceThunk(value fn() Int) Int =
    value() + value()

twice(expensive())
twiceThunk(() => expensive())
```

Use by-name parameters for conditional-value APIs. Use `fn() T` when a
caller passes, stores, returns, or explicitly invokes the work itself.

## Higher-Order Types

The `fn` marker keeps nested function types readable:

```txt
factory fn() fn(Int) Str
transform fn(Int) fn(Str) Bool

def compose(
    first fn(Int) Str,
    second fn(Str) Bool
) fn(Int) Bool =
    value => second(first(value))
```

## Declaration Disambiguation

Named function declarations begin with `def`, and their parameters should have
explicit types. Lambda parameters may continue to infer their types.

```txt
def sum(left Int, right Int) Int =
    left + right

pair (Int, Int) =
    (1, 2)
```

The `def` marker lets the parser distinguish a named function declaration from
a tuple-typed binding without inspecting the declaration's parameter grammar.
Zero-argument forms are equally explicit:

```txt
def now() Int = 0
nowRef fn() Int = now
```

The language should not rely on a `name(...)` versus `name (...)` distinction.

## Migration

Do not retain old arrow-based function-type spellings. Replace:

```txt
(Int) => Str
fn(Int) => Str
```

with:

```txt
fn(Int) Str
```

Lambda expressions and match branches continue to use `=>` without `fn`:

```txt
mapper fn(Int) Str =
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
def getCurrentTime() Int =
    0

# Function-valued binding
getTimeRef fn() Int =
    getCurrentTime

# Function-valued parameter
def run(operation fn() Unit) Unit =
    operation()

# Function-valued return
def makeAdder(base Int) fn(Int) Int =
    value => base + value

# By-name parameter
def getOr(defaultValue => Int) Int =
    ...

# Lambda expression
increment =
    value => value + 1
```

The language model is therefore:

- require `def` on named functions and methods
- require `fn(...) T` for function types
- keep arrow-only lambda expressions
- keep constructors distinct with `new(...)`, without `def`
