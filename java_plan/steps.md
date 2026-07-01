# Executable Java Backend Steps

## Step 1: Backend Seam

Status: done.

Goal: add the smallest backend path that reuses the existing checked/lowered Lume pipeline and branches before interpretation.

Tasks:

- Add target-neutral `backend` module.
- Add `BackendBundle` built from the same path used by `lume run`.
- Add placeholder descriptors derived from lowered IR.
- Add Java backend module that consumes `BackendBundle`.
- Add `lume java <file> --out <dir>`.
- Emit one marker Java source file.
- Verify invalid Lume does not generate Java.

Done when:

- `lume java examples/hello.lum --out /tmp/lume-java` writes a Java marker file.
- `cargo test --manifest-path rust/Cargo.toml -p lume` passes.

## Step 2: Declaration Skeletons

Status: done.

Goal: emit Java source skeletons for Lume declarations.

Tasks:

- Generate package paths.
- Generate module wrapper classes using `Module` suffix.
- Generate class shells.
- Generate shape records.
- Generate single shells.
- Generate interface shells.

## Step 3: Runtime Library Skeleton

Status: done.

Goal: add the Java runtime package that generated code can target.

Tasks:

- Add `java_runtime/src/main/java/lume/runtime`.
- Add `LumeUnit`, `Option`, `Result`, `Either`, tuple classes, `LumeList`, `LumeArray`, `LumeSet`, `LumeMap`, `Range`, and `LumePanic` shells.

## Step 4: Body Codegen MVP

Status: done for MVP-supported IR; unsupported complex IR still falls back to explicit stubs.

Goal: generate correctness-first Java bodies from IR.

Tasks:

- Emit locals.
- Emit assignments.
- Emit calls.
- Emit returns.
- Emit block-state control flow.

## Step 5: Parity Harness

Status: done for MVP-supported Java generation; the test skips when `javac`/`java` are unavailable.

Goal: compare interpreter output and generated Java output for selected examples.

Tasks:

- Generate Java into a temp folder.
- Compile with `javac`.
- Run with `java`.
- Compare output with interpreter.

## Step 6: Java External Resolver

Goal: let ordinary Lume imports resolve Java classes into Lume-shaped external descriptors.

Tasks:

- Add external resolver registry.
- Add Java classpath resolver.
- Add optional descriptor file fallback.
- Resolve external constructors, methods, fields, enum constants, and static members before codegen.
