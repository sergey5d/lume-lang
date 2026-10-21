# `ext` on interfaces: dispatch through an interface is unspecified and unsound

Observed at commit `f208f747`, using the interpreter (`lume run`) and the Java
backend (`lume gen` followed by `javac`).

## Summary

`ext Interface { ... }` adds methods to an interface. When a class that
implements the interface declares a member with the same name, a call through
the interface type runs the **class** member, not the `ext` method. The reader
cannot tell which body runs without knowing the runtime type. That is exactly
the behaviour-inheritance problem Lume avoids by having no class inheritance.

`syntax.md` does not say what happens when an `ext` member and a class member
share a name, so every result below is unspecified behaviour.

| # | Problem | Severity |
|---|---------|----------|
| 1 | A class member silently overrides an `ext` method when called through the interface | Design: hidden dispatch |
| 2 | The overriding member may have a different return type, which breaks type soundness | Bug: runtime crash after a clean `check` |
| 3 | `ext Interface` methods are not visible on concrete classes or bounded generics | Inconsistency |
| 4 | The Java backend emits `ext` methods as abstract interface methods, so the generated code does not compile | Bug: backend |

## 1. A class member silently overrides an `ext` method

```txt
interface Shape {
    def area() Float
}

ext Shape {
    def describe() Str = "shape with area ${this.area()}"
    def report() Str = "report: ${this.describe()}"
}

class Square with Shape {
    side Float
    def area() Float = this.side * this.side
    def describe() Str = "square"          # same name as the ext method
}

class Circle with Shape {
    radius Float
    def area() Float = 3.0 * this.radius * this.radius
}

def viaInterface(s Shape) Str = s.describe()
def reportOf(s Shape) Str = s.report()
```

Interpreter output:

```txt
sq.describe()                → square
viaInterface(sq)             → square                 # class member wins through Shape
asShape.describe()           → square
reportOf(sq)                 → report: square         # ext → ext call also dispatches to the class
viaInterface(Circle { .. })  → shape with area 3.0    # ext body runs only when no class member exists
```

Reading `viaInterface`, you would expect `ext Shape.describe` to run. Which body
actually runs depends on the runtime class. `Square` never declares that it
replaces the shared behaviour, and nothing at the call site shows that it does.

This is override semantics without an `override` keyword or any other visible
marker, which is behaviour inheritance in all but name.

## 2. An overriding member with a different signature breaks soundness

```txt
interface Shape {
    def area() Float
}

ext Shape {
    def describe() Str = "shape"
}

class Square with Shape {
    side Float
    def area() Float = this.side * this.side
    def describe() Int = 42                # different return type
}

def main() Unit {
    s Shape = Square { side: 1.0 }
    text Str = s.describe()                # typechecks as Str
    println(text.toUpper())
}
```

- `lume check` passes with exit code 0.
- `lume run` fails with: `runtime_error: method 'toUpper' is not available on 42`.

The static type is `Str`, but the runtime value is `Int`. This is the same kind
of hole as the earlier problem of user code impersonating primitive types:
something the type checker accepts crashes at runtime.

## 3. `ext Interface` methods are visible only through the interface type

Using the same `Shape` and `ext Shape`:

```txt
Circle { radius: 1.0 }.describe()
# error[unknown_member]: type 'Circle' has no field or method 'describe'

def viaGeneric[T with Shape](s T) Str = s.describe()
# error[unknown_member]: type 'T' has no field or method 'describe'
```

So whether `describe()` can be called depends on the static type of the receiver:

| Receiver | `describe()` |
|----------|--------------|
| `Shape` | ext method, or the class member if one exists (see 1) |
| `Circle`, no own member | not found |
| `Square`, own member | the class member |
| `T with Shape` | not found |

The case of a bounded generic is particularly surprising, because `T with Shape`
is the main way to write code over "anything that is a `Shape`".

## 4. Java backend: `ext` methods are emitted as abstract interface methods

For the example in section 1, `lume gen` produces:

```java
interface Shape extends LumeTyped {
    ...
    Double area();
    String describe();   // ext body is lost
    String report();     // ext body is lost
}
```

`javac` then fails on both classes, including `Circle`, which does not declare
`describe` or `report`:

```txt
Circle.java:11: error: Circle is not abstract and does not override abstract method report() in Shape
Square.java:11: error: Square is not abstract and does not override abstract method report() in Shape
```

The runtime `TYPE` metadata for `Shape` also lists `describe` and `report` as
interface methods. Reflection therefore reports `ext` members as part of the
contract, while the interpreter does not.

## Decision needed: what does `ext Interface` mean?

### Option A: `ext` is shared behaviour that cannot be overridden (recommended)

- `ext Interface` methods are built only from the interface's own members.
- **A class that implements the interface cannot declare a member with the same
  name as an `ext` method on that interface.** That is a compile error, whatever
  the signature.
- Every call to an `ext` method runs exactly the `ext` body, so you can find the
  code from the static type alone.
- `ext` methods are visible on every value whose static type implements the
  interface: the interface type, concrete classes and `T with Interface`.
- If an implementation needs its own version, for example a fast `size` on
  `Vector` instead of counting through `iterator()`, the member belongs in the
  interface as an abstract member that every class implements explicitly.

This makes "nothing inherits behaviour" literally true and keeps `ext`
consistent with the "nothing hidden" goal.

If interface default methods are dropped later, Option A is their replacement.
Shared behaviour moves to `ext`, and interfaces become pure contracts.

### Option B: `ext` methods are overridable defaults

Keep the current dispatch, but specify it:

- A class may override an `ext` method only with a compatible signature. That
  fixes problem 2.
- Overriding should be marked explicitly at the declaration (for example with
  `override`), so the reader can see it.
- `ext` methods should still be visible on concrete classes and bounded
  generics. That fixes problem 3.

This is interface default methods under another name. It keeps the
"which body runs?" question that Option A removes.

### Option C: static dispatch, and the class member shadows only on the concrete type

A call through `Shape` always runs the `ext` body, and a call on `Square` runs
the class member.

Not recommended. The same expression gives different results depending on the
static type of the receiver, which is the classic confusion around extension
methods in C# and Kotlin.

## Work needed for Option A

1. **Typecheck:** reject a class member that shares a name with an `ext` method
   of an interface the class implements. Report it on the class member and name
   the `ext` declaration.
2. **Typecheck:** resolve `ext Interface` methods on concrete implementing types
   and on generics bounded by the interface.
3. **Interpreter:** dispatch `ext` methods statically to the `ext` body.
4. **Java backend:** emit `ext Interface` methods as static helpers, or as
   `default` methods that implementing classes are guaranteed not to override.
   Do not emit them as abstract members. Keep them out of the interface's
   reflected `TYPE` method list, or mark them as extensions there.
5. **`syntax.md`:** add the rule to the `ext` section and to the visibility and
   member-resolution rules.
6. **Examples:**
   - `examples/failures/` for the name clash.
   - `examples/features/` for `ext` calls on concrete, interface and generic
     receivers.
   - A Java-backend test that compiles and runs the section 1 program.
