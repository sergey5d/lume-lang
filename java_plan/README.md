# Java Backend Plan

## Goal

Add a Java source backend for Lume.

The backend should:

- Generate Java packages and Java source files from checked `.lum` files.
- Reuse the existing Lume frontend validation, so Java generation never becomes a second typechecker.
- Allow Lume code to call Java classes, methods, fields, constructors, enums, and static members through a constrained Java import model.
- Generate Java classes from Lume declarations, but not try to generate Lume declarations from Java source.

The first rule is important:

```txt
if Java generation starts, the Lume program is already resolved, typechecked, and lowerable
```

So codegen errors should mostly be backend limitations, unsupported constructs, or invalid external descriptors. They should not be ordinary language validation errors.

## Architecture Principle

Use as much of the existing Lume implementation as possible.

The long-term shape should be:

```txt
shared Lume frontend
-> shared resolver/typechecker/lowerer
-> shared backend bundle
-> target generator
```

Target-specific code should be narrow:

- external resolvers know how to load the surrounding world, such as Java classes
- generators know how to emit one target, such as Java source or later Java bytecode

The shared compiler should see external libraries through Lume-shaped descriptors, similar to loaded Lume modules. A generator should not own knowledge of external class shapes or module loading.

## Settled Backend Decisions

- Use boxed Java types in the first backend pass.
- Target the most recent Java version available to the build environment.
- Generate Lume `shape` declarations as Java records.
- Generate module wrapper classes with a `Module` suffix, for example `math` -> `MathModule`.
- Java external imports use ordinary Lume path syntax, for example `use java/time/Instant`.
- External Java descriptors are the checked shapes of Java classes: constructors, methods, fields, constants, type parameters, and canonical target names. These descriptors should normally come from classpath inspection; hand-written descriptor files are only an optional fallback/cache.
- Java exceptions do not surface in Lume. Generated Java must catch Java exceptions at the call boundary when a backing Java call can throw.
- First-pass Java output should prioritize correctness over readability. Block-state code is acceptable for any nontrivial body.

## Non-Goals

- Do not execute Java directly from the current Rust interpreter.
- Do not compile Java bytecode in the first version.
- Do not parse arbitrary Java source.
- Do not implement full Java overload resolution from scratch beyond the external descriptors we explicitly load.
- Do not support extending arbitrary Java classes at first.
- Do not support Java calling generated Lume code through a polished API until Lume-to-Java generation is stable.

## Current Pipeline

Important: Java generation is not a second parser/typechecker/compiler frontend.
It reuses the existing Lume pipeline and swaps the final execution step.

Today, `lume run` does this:

```txt
source files
-> lexer.rs
-> parser/*
-> ast.rs
-> resolver.rs
-> typecheck.rs
-> desugar.rs / core.rs
-> lower.rs / ir.rs
-> runtime/types.rs
-> interpreter.rs
```

The Java command should do the same shared frontend/lowering work, then call a
generator instead of the interpreter:

```txt
source files
-> lexer.rs
-> parser/*
-> ast.rs
-> module graph + external import resolvers
-> resolver.rs
-> typecheck.rs
-> desugar.rs / core.rs
-> lower.rs / ir.rs
-> backend descriptors
-> java_backend/*
-> generated .java files
```

Generation should start after the same semantic checks used by `lume check`.

## Proposed File Layout

```txt
rust/crates/lume/src/
  backend/
    mod.rs              shared backend entry points
    descriptors.rs      checked Lume-facing descriptors for modules, types, methods, fields
    externals.rs        external library resolver traits and registry
    bundle.rs           backend-ready program bundle assembled after check/lower
    capabilities.rs     target capability flags and unsupported-feature reporting
    diagnostics.rs      backend-agnostic diagnostics
  java_backend/
    mod.rs              Java source backend entry points
    model.rs            Java source model: package, class, method, field, stmt, expr
    names.rs            Lume-to-Java name mangling and reserved-word escaping
    types.rs            Lume type -> Java type mapping
    lower.rs            backend descriptors/IR -> Java source model
    emit.rs             Java source text emission
    diagnostics.rs      backend-specific diagnostics
```

The split is intentional:

- `backend/` is target-neutral plumbing.
- `backend/externals.rs` knows how to ask registered external resolvers for imported library shapes.
- `java_backend/` only turns checked Lume descriptors and IR into Java source.
- A future bytecode backend or another source backend should consume the same `backend::Bundle`.
- No parser, AST, resolver, or typechecker should be duplicated under `java_backend/`.
- For Java generation, think "replace `interpreter.rs` with `java_backend` after lowering," not "build a parallel Java compiler."

CLI addition:

```txt
lume gen <file> --out <dir>
```

Start with one command only.

## Backend Input

Use the same validated path as execution, stopping where `run` would enter the interpreter:

1. `check_path(path)`
2. `load_module_graph(path)`
3. resolve external imports into Lume-shaped descriptors
4. merge reachable Lume modules for backend input
5. `lower_program(&program)`
6. assemble a backend bundle from IR plus checked descriptors
7. generate Java from the backend bundle

The Java backend should not parse or typecheck Lume again.

The Java backend also should not discover Java class shapes directly. Java class discovery belongs to an external resolver that runs before typechecking/generation and returns Lume internal descriptors.

Backend input policy:

- Use IR as the primary backend input.
- Avoid pulling information from AST whenever the same information can live in IR.
- If Java generation needs declaration/package/annotation facts that are currently only in AST, prefer enriching IR or producing a small checked backend descriptor structure during lowering.
- Treat direct AST reads as a temporary escape hatch, not the backend model.

This avoids reimplementing control-flow lowering and keeps Java generation closer to the small execution model we already trust.

## Backend Descriptor Model

Add a target-neutral checked descriptor layer that sits between resolver/typecheck/lowering and concrete generators.

The descriptor layer should contain Lume-shaped facts, not Java-shaped facts:

```txt
BackendBundle
  modules
  types
  functions
  globals
  lowered IR program
  external descriptors
```

Useful descriptor shapes:

```txt
DescriptorModule
DescriptorType
DescriptorField
DescriptorMethod
DescriptorConstructor
DescriptorEnumCase
DescriptorSingle
DescriptorInterface
DescriptorExternalSymbol
```

Important rule:

```txt
generators consume descriptors and stable refs, not classpath discovery results
```

That means generated Java calls an external Java method because the checked program contains a resolved external method ref. The generator should not ask, "what methods does this Java class have?" during emission.

This keeps the surrounding-world knowledge out of codegen and makes future targets possible:

- Java source backend
- Java bytecode backend
- another source-language backend
- another external library resolver

## External Resolver Model

External library support should look similar to loading another Lume module.

Suggested trait shape:

```txt
ExternalResolver
  canResolve(importPath) -> Bool
  resolveImport(importPath) -> external descriptors
```

Example flow:

```txt
use java/time/Instant

parser records an external import
external resolver loads java.time.Instant descriptors
resolver/typechecker sees a Lume-shaped external class descriptor
lowering records stable refs to external constructors/methods/fields
Java generator emits java.time.Instant calls from those refs
```

For Java, an external descriptor should include:

- canonical Java name
- Lume-facing type name
- type parameters
- constructors
- instance methods
- static methods
- fields
- enum constants
- interface information

But after that descriptor is loaded, the rest of the compiler should treat it like a checked external type, not like raw Java reflection data.

## Package Mapping

Lume modules map to Java packages.

```txt
module app/domain/user
```

generates:

```java
package app.domain.user;
```

If a file has no `module`, derive the package from either:

- a CLI base package option, or
- the relative path from the source root.

First version should require an explicit package root if module names are missing:

```txt
lume gen examples/hello.lum --out generated --base-package lume.generated
```

## Java Target Version

Target the most recent Java version available to the build environment. The backend does not need to stay compatible with old Java releases.

This lets us use records for shapes and modern Java constructs when they simplify generation.

## Declaration Mapping

### Classes

Lume:

```txt
class User {
    name Str
    age Int = 0
}
```

Java:

```java
final class User {
    final String name;
    final Long age;

    User(String name) {
        this.name = name;
        this.age = Long.valueOf(0L);
    }

    User(String name, Long age) {
        this.name = name;
        this.age = age;
    }
}
```

Java access modifiers are a backend policy. The first plan keeps examples package-local so Lume visibility wording stays separate from Java syntax details.

### Mutable Fields

Lume `var` fields become non-final Java fields.

```txt
var count Int
```

becomes:

```java
long count;
```

### Hidden Fields and Members

Lume `hidden` maps to Java `private`.

Within the same generated class, hidden members can be used normally.

Cross-module hidden access must already be rejected by resolver/typecheck before codegen.

### Shapes

Lume shapes generate Java records.

Lume:

```txt
shape Point {
    x Int
    y Int
}
```

Java:

```java
record Point(Long x, Long y) {}
```

### Singles

Lume:

```txt
single Config {
    host Str = "localhost"
}
```

Java:

```java
final class Config {
    static final Config INSTANCE = new Config();

    final String host = "localhost";

    private Config() {}
}
```

Lume expression `Config` lowers to `Config.INSTANCE`.

### Enums

Zero-payload cases can map to Java enum constants.

Payload cases need a sealed hierarchy or tagged class.

Preferred first version:

- Generate all Lume enums as sealed-ish class hierarchies if target Java version supports it.
- If not, generate a tagged abstract base plus nested final subclasses.

Example:

```java
abstract class Option<T> {
    private Option() {}

    static final class Some<T> extends Option<T> {
        final T value;
        Some(T value) { this.value = value; }
    }

    static final class None<T> extends Option<T> {
        None() {}
    }
}
```

### Interfaces

Lume interfaces map to Java interfaces.

Default methods can become Java default methods when the body is backend-supported.

### Top-Level Functions and Constants

Java has no true top-level members. Generate a module class:

```txt
module app/math

def add(a Int, b Int) Int = a + b
seed Int = 1
```

becomes:

```java
package app;

final class MathModule {
    static final Long seed = Long.valueOf(1L);

    static Long add(Long a, Long b) {
        return a + b;
    }

    private MathModule() {}
}
```

Module wrapper names use the module basename converted to PascalCase plus `Module`. Name mangling handles collisions and Java reserved words.

## Type Mapping

Only primitive-like values and `Unit`/Java `void` need direct special-case mapping. Everything else should be a generated Lume class, an external Java class, or a Lume Java runtime class.

Initial primitive mapping:

```txt
Lume          Java
Unit          void for returns, LumeUnit for values if needed
Bool          Boolean
Int           Long
Float         Double
Str           String
Rune          Integer
```

Boxing rule:

- Use boxed types in the first backend pass.
- Primitive Java locals can be an optimization later.

Non-primitive Lume type mapping:

```txt
Lume          Java
[T]           lume.core.LumeList<T>
List[T]       lume.core.LumeList<T>
Array[T]      lume.core.LumeArray<T>
Set[T]        lume.core.LumeSet<T>
Map[K, V]     lume.core.LumeMap<K, V>
Option[T]     lume.core.Option<T>
Result[T, E]  lume.core.Result<T, E>
Either[L, R]  lume.core.Either<L, R>
Tuple2        lume.core.Tuple2<A, B>
TupleN        lume.core.TupleN<...>
shape         generated value class
class         generated nominal class
single        generated singleton class with INSTANCE
interface     Java interface
function      generated functional interface or java.util.function where useful
```

Collection implementation rule:

- Lume collections should be represented by our own Java runtime classes.
- `LumeList`, `LumeSet`, and `LumeMap` may be backed by `ArrayList`, `HashSet`, `HashMap`, or similar Java standard library types internally.
- Generated Lume code should call methods on the Lume runtime classes directly.
- Do not create a broad method-mapping table for `List`, `Set`, and `Map`; the runtime classes should expose the Lume method surface.
- Keep mapping tables only for primitive-like operations and Java interop boundaries where there is no Lume-owned receiver class.

## Runtime Support

Generated Java will need a small Java runtime library.

Proposed package:

```txt
lume.core
```

Minimum runtime types:

- `LumeUnit`
- `Option<T>`
- `Result<T, E>`
- `Either<L, R>`
- `Tuple2`, `Tuple3`, ...
- `LumeList<T>`
- `LumeArray<T>`
- `LumeSet<T>`
- `LumeMap<K, V>`
- `Range`
- `LumePanic`
- string/rune helpers

Collection runtime classes should own their method implementations. Internally they can delegate storage to standard Java collections, but that should be hidden behind the Lume runtime API so later we can replace implementations without changing generated code.

This can live under:

```txt
lume/core/src/main/java/lume/core/
```

or be emitted into the output folder for the first MVP.

Preferred first version:

- keep checked-in Java runtime substrate source under `lume/core/src/main/java/`
- generated code imports `lume.core.*`

## Java Imports Into Lume

We need Lume to understand Java symbols before generation. That should happen through the external resolver layer, not through Java codegen.

Java imports use normal Lume path syntax:

```txt
use java/time/Instant
use java/util/ArrayList
use com/example/Foo
use com/example/Foo/{bar, Baz}
```

The import path maps to the canonical Java name by joining path segments with dots:

```txt
use java/time/Instant  -> java.time.Instant
```

No extra namespace marker is needed. The Java external resolver gets a chance to resolve import paths that are not found as `.lum` modules.

### Java External Descriptors

Do not parse Java source in the compiler. The Java external resolver loads Java type descriptors from:

1. classpath inspection whenever possible
2. optional descriptor files as a fallback/cache

Descriptor files are not a new semantic layer; they are just one way to provide the shape of external Java classes when classpath inspection is not available yet.

Possible fallback file layout:

```txt
java_imports/
  java.util.ArrayList.lumejava
  java.time.Instant.lumejava
```

Example descriptor file:

```txt
java class java.time.Instant {
    static def now() Instant
    def toString() Str
}
```

The descriptor file syntax can be Lume-like, JSON, or TOML. It is only tooling input to the Java external resolver.

Resolver output should not be raw Java descriptor data. It should be normalized into backend descriptors:

```txt
classpath / descriptor files
-> Java external resolver
-> Lume-shaped external descriptors
-> resolver/typechecker/lowering refs
-> backend bundle
-> Java source generator
```

### Mapping Java Object Model To Lume

Java class:

- maps to a nominal external class type
- constructors map to callable constructor shapes
- instance methods map to Lume methods
- static methods map to singleton-side methods or qualified functions
- static fields map to immutable imported values when final
- enum constants map to stable singleton-like values

Java interfaces:

- map to Lume interfaces when method signatures are representable

Java generics:

- support invariant generic class references first
- ignore wildcards initially unless descriptors normalize them

Java overloads:

- resolve using the same call-resolution shape as Lume overloaded methods
- reject ambiguous overloads

Java exceptions:

- Java exceptions do not become a Lume language feature.
- Generated Java methods should not add Java `throws` clauses for Lume-visible APIs.
- If a backing Java call can throw, generated Java catches it at the call boundary.
- Lume runtime helpers can translate Java exceptions into Lume values when that is the Lume API. For example, an `Int.parse` helper backed by `Integer.parseInt` catches `NumberFormatException` and returns `None`.
- For raw external Java calls without a Lume-level failure value, generated Java catches and converts to `LumePanic`.

This mapping belongs to the Java external resolver and shared descriptor assembly. The Java source generator should only see that a call target is an external static method, external instance method, external constructor, or external field with a canonical target name.

## Code Generation Strategy

### MVP Body Generation

Use IR and emit correctness-first Java. The first pass does not need pretty Java.

Block-state code is acceptable for nontrivial control flow:

```java
int block = 0;
while (true) {
    switch (block) {
        case 0:
            ...
            block = 1;
            continue;
        case 1:
            return value;
    }
}
```

This is less pretty, but it keeps the first backend simple and correct.

Later, optionally add structured Java emission for:

- simple `if`
- `while`
- `for`
- `match`
- expression-bodied methods

### Name Mangling

Need deterministic escaping for:

- Java reserved words
- Lume operator methods
- overloaded methods
- generated temp locals
- module wrapper classes
- enum case classes

Proposed pattern:

```txt
class         -> class_
operator +   -> op_plus
operator []  -> op_index
tmp          -> _lume_tmp_0
```

### Source Files

Generate one `.java` file per Java top-level class.

Possible outputs:

```txt
generated/
  app/domain/User.java
  app/domain/UserModule.java
  lume/core/Option.java
```

## Validation Boundary

Before codegen:

- syntax is valid
- imports resolve
- hidden visibility is enforced
- names resolve
- types are valid
- overloads are resolved or rejected
- constructors are valid
- control flow is valid
- `try`, `expect`, `let`, `for`, `match`, lambdas, lifted access are checked
- external imports are resolved into descriptors
- external calls are typechecked against descriptors

During codegen:

- report unsupported backend constructs
- report unsupported external descriptor shapes for the chosen backend
- report name collisions after mangling
- report Java target-version limitations

Codegen should not invent new semantic rules.

## Implementation Phases

### Phase 1: Shared Backend Bundle Skeleton

Deliverables:

- `backend/descriptors.rs`
- `backend/bundle.rs`
- `backend/externals.rs`
- target-neutral `BackendBundle`
- `java_backend/mod.rs`
- `JavaBackendOptions`
- `JavaBackendResult`
- CLI command `lume gen <file> --out <dir>`
- output directory creation
- no-op generation for an empty/simple file
- tests that invalid Lume does not generate Java

Acceptance:

- `lume gen examples/hello.lum --out /tmp/lume-java` runs through check/lower
- backend can emit at least one placeholder Java source file
- Java generation receives a `BackendBundle`, not raw AST-only state

### Phase 2: Names, Packages, and Declarations From Descriptors

Deliverables:

- package mapping
- name mangling
- generated classes for Lume `class`, `shape`, `single`, `interface`
- generated module wrapper for top-level functions/constants
- generated fields and constructors

Acceptance:

- simple Lume declarations generate compilable Java source skeletons
- no method bodies beyond trivial constants required yet
- declaration generation reads checked descriptors and lowered ids

### Phase 3: Runtime Library MVP

Deliverables:

- checked-in Java runtime package
- `Option`, `Result`, `Either`, tuples, unit, panic
- collection helper interfaces/classes needed by generated code

Acceptance:

- generated Java can compile with runtime classes on classpath

### Phase 4: Function and Method Body Codegen

Deliverables:

- IR statement/terminator emission
- local variables
- assignments and reassignments
- calls and method calls
- constructors
- conditionals and loops through block-state Java
- returns

Acceptance:

- generated Java for basic examples compiles and produces same output as interpreter

### Phase 5: Primitive and Runtime Type Mapping

Deliverables:

- direct mapping for `Unit`, `Bool`, `Int`, `Float`, `Str`, and `Rune`
- runtime class references for `List`, `Array`, `Set`, `Map`, `Option`, `Result`, `Either`, tuples, and `Range`
- generated calls to Lume runtime class methods for collection behavior
- indexing and collection operations

Acceptance:

- collection/string-heavy examples compile and match interpreter output

### Phase 6: Lambdas and Higher-Order Functions

Deliverables:

- generated functional interfaces or Java lambdas
- closure capture support
- `map`, `flatMap`, `filter`, `reduce`, `sort`

Acceptance:

- lambda and collection HOF examples compile and match interpreter output

### Phase 7: Pattern Matching and Algebraic Values

Deliverables:

- enum payload class generation
- `match` lowering through `instanceof`/tag checks
- `partial match`
- `let`/`expect` destructuring support

Acceptance:

- enum, option/result/either, destructuring examples compile and match interpreter output

### Phase 8: Java External Resolver

Deliverables:

- parser support for Java import declarations
- external resolver registry
- Java resolver that turns classpath data or descriptor files into shared external descriptors
- resolver/typechecker support for external descriptors
- lowering refs for external classes, constructors, methods, fields, and static members
- optional descriptor file format

Acceptance:

- Lume can call a descriptor-declared Java static method and instance method
- generated Java imports and calls the real Java class from already-resolved external refs

### Phase 9: Java Classpath Discovery

Deliverables:

- optional Java external resolver mode that reads `.class`/classpath reflection
- CLI option for classpath

```txt
lume gen app.lum --out generated --classpath libs/foo.jar
```

Acceptance:

- simple JDK classes can be referenced without hand-written descriptor files

### Phase 10: Parity Test Harness

Deliverables:

- test command that:
  - runs interpreter output
  - generates Java
  - compiles Java with `javac`
  - runs Java
  - compares output

Acceptance:

- selected examples pass interpreter-vs-Java parity

## Suggested First PR

Keep the first PR boring and safe:

1. Add shared `backend` module with descriptor and bundle scaffolding.
2. Add `java_backend` module with options/result types.
3. Add `lume gen <file> --out <dir>` CLI command.
4. Reuse `check_path`, module loading, and `lower_program`.
5. Emit one generated marker file containing package and source comments.
6. Add tests proving invalid Lume prevents generation.

No Java interop yet. No full class emission yet.

This gives us the backend seam without destabilizing the language frontend.

## Remaining Follow-Ups

- Decide the optional descriptor file format only if classpath inspection is not enough for early Java interop.
- Decide exact Java bytecode backend shape later; it should consume the same backend bundle.
