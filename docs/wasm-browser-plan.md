# Browser WebAssembly Plan

Status: Milestone 1 is implemented in `rust/crates/lume-web` and `web/`.
See [browser setup, SDK, and safety contract](../web/README.md). The direct
code-generation backend described as Milestone 2 below is not implemented and
is outside the current scope. No Lume syntax changes are required.

## Goals And Delivery Order

1. Run, check, and format Lume entirely in a browser, without a compilation
   server, JVM, or installed toolchain.
2. Expose a small JavaScript/TypeScript SDK for embedding that functionality.
3. Subsequently compile Lume programs into executable WebAssembly, rather than
   interpreting their IR.

These are two different products. Compiling the Rust interpreter to Wasm makes
Lume usable in the browser, but does not compile each Lume program to Wasm.
The first product is the recommended initial milestone. Its source-loading,
host, diagnostics, and test infrastructure should also support the second.

## Existing Foundations

- The parser, checker, Core lowering, and typed IR are already Rust libraries.
- `backend/bundle.rs` assembles checked, merged IR and backend metadata, although
  its entry point currently takes a filesystem path.
- `stdlib/declarations/` supplies Rust-authored contracts independently of
  external `.lum` files. Native implementation bindings already have a registry.
- `run_program_entry_with_args` executes IR in memory; `run_path_with_args`
  still performs filesystem-oriented checking and module loading.
- `interpreter.rs` currently owns runtime values, file handles, native host
  calls, and execution together. Its values use `Rc`/`RefCell`; those layouts
  are implementation details, not a suitable public Wasm ABI.
- `BackendCapabilities` is a starting point, but is not yet a complete browser
  availability policy.

## Milestone 1: Browser Execution

### Step 1: Establish A Wasm Build

Add `rust/crates/lume-web` as a small `cdylib` wrapper over the existing library.
Target `wasm32-unknown-unknown`, use `wasm-bindgen` for JavaScript bindings, and
keep browser dependencies out of the native CLI's default build.

Start with a compile-only portability spike. Audit all dependencies and
feature-gate native CLI/Java tooling where necessary; do not assume that a
successful build makes filesystem or process APIs usable in a browser.

Acceptance: load the module in a real browser and parse, check, lower, and run a
small in-memory program that prints an integer. No server-side execution.

### Step 2: Share An In-Memory Project Pipeline

Introduce a source-provider abstraction with filesystem and in-memory
implementations. A project consists of an entry module plus a map of virtual
paths to source text. Normalize paths and resolve imports without OS
canonicalization, ancestor-directory searches, or environment variables.

Build one checked module graph and pass it to merging/lowering. Native path
APIs become adapters over that shared pipeline rather than maintaining a
separate browser checker. Preserve module visibility, import aliases,
initialization order, source identities, and diagnostic locations.

Native stdlib declarations remain embedded. Bundle any additional source-only
library modules deliberately. Do not perform automatic network imports.

Acceptance: the same multi-file project produces the same results and
diagnostics through filesystem and virtual-source providers, including errors
in imported files. Browser checking never consults `LUME_STDLIB`.

### Step 3: Define The Browser Host Boundary

Separate platform services from ordinary native implementations. Keep one
stdlib contract; associate bindings with required target/host capabilities.
Reject unavailable operations before running, instead of leaking unsupported
`std` calls or silently returning placeholder values.

| Facility | Initial browser policy |
| --- | --- |
| Numeric types, strings, collections, JSON, regex | Reuse Rust implementations; verify parity |
| Printing | Capture output; stream bounded batches to the worker client |
| `OS.args` | Values explicitly supplied by the embedding application |
| `OS.env` | Explicit host-supplied map, empty by default; no machine secrets |
| Native files, processes, sockets, Java/JAR interop | Unsupported-target diagnostic |
| Browser files, DOM, network | Later explicit adapters, not implicit access |

Browser-selected files may later be read asynchronously by JavaScript and
mounted as bytes in a virtual filesystem. That can support synchronous reads
of already-mounted content without disguising asynchronous I/O as synchronous.
Do not add blocking `fetch`, synchronous DOM access, or hidden callback wrapping.

Acceptance: pure stdlib fixtures pass; unavailable calls identify the member
and missing browser capability. No Java stub tooling is invoked.

### Step 4: Add Worker Isolation And Execution Limits

Compile and execute inside a dedicated Web Worker, not the UI thread. Use a
versioned request/response protocol with request IDs. A JavaScript caller sees
asynchronous SDK methods even though individual Lume programs remain
synchronous.

Add interpreter fuel, maximum call depth, output bounds, and limits on large
collection/string/decimal operations. Also bound source size, module count, and
compiler nesting. Fuel must account for costly native operations, not just IR
branches. Use a bounded cleanup allowance when language-managed unwinding runs
defers after a limit error.

For the first release, each run gets a disposable Wasm instance/worker. This
also reclaims any reference cycles left by the current `Rc` runtime. A main-
thread watchdog can terminate the worker for a stuck compiler or native call.
It cannot depend on the blocked worker receiving a cancellation message.

Ordinary Lume errors remain diagnostics and follow Lume cleanup semantics.
Worker termination, out-of-memory traps, and unexpected Rust panics are fatal
session failures: discard the instance and do not promise cleanup. A worker
and Wasm are not a complete CPU/memory sandbox by themselves; imports remain
minimal and explicitly granted.

Acceptance: an infinite loop, recursion, output flood, large allocation, and
fatal trap cannot freeze the editor; a subsequent fresh run still works.

### Step 5: Ship The SDK And Playground

Proposed SDK operations: `check(project)`, `format(source)`, and
`run(project, options)`. Include entry selection, supplied args/environment,
limits, structured diagnostics, output events, and a final result. Cancellation
belongs to the worker client and initially means terminating the active run.

Keep the initial return value as display text, as today's interpreter API does.
Do not claim arbitrary Lume objects round-trip through JSON. Preserve UTF-8
byte spans internally and convert to the editor's UTF-16 offsets at the UI
boundary. Separate warnings, language errors, limits, and fatal host failures.

Add a small `web/` SDK/worker package and playground with Run, Stop, Format,
examples, output, and clickable diagnostics. Serve it as static assets. Cache
the Wasm download separately from per-run instances, and measure compressed
size, startup cost, and check/run latency before optimizing.

Acceptance: a static-hosted, multi-file playground runs without a backend,
survives repeated errors/cancellation, and passes headless Chromium, Firefox,
and WebKit tests plus the native CLI regression suite.

## Milestone 2: Direct Lume-To-Wasm Compilation

This is a substantial backend project, not another wrapper around the
interpreter. Keep it separately gated from the browser playground milestone.

### Step 6: Fix The Runtime ABI And Memory Policy

Specify a versioned ABI before generating object operations. Prefer linear
memory plus runtime-owned handles initially, reusing portable Rust library
algorithms without exposing Rust enum, pointer, or `Rc` layouts. Define object
lifetime, roots, release/disposal, and cycles explicitly. A disposable arena is
adequate only for bounded runs; long-lived browser applications require a
cycle-safe collection strategy before being advertised as production-ready.

Define strings, arrays/collections, shapes, class identity, union tags,
closures/captures, interface dispatch, and reified metadata. Start with a boxed
generic ABI and selectively specialize primitives, not an assumption that every
generic can be monomorphized trivially. Share a stable runtime ABI between the
generated module and its runtime; package both with a loader initially rather
than requiring a bespoke linker immediately.

Preserve these language contracts:

- `Int` is signed 64-bit with checked ordinary arithmetic, not wrapping Wasm
  arithmetic; `Float` is binary64. Guard overflow and division edge cases.
- `Decimal` uses the existing exact decimal implementation, never `f64`.
- Unicode scalar indexing, equality/hashing, shallow shape construction,
  collection aliasing, constructor/mixin order, and written evaluation order
  remain unchanged.
- Language errors and `defer` need explicit failure propagation/cleanup paths;
  a raw Wasm trap is not an implementation of language-managed unwinding.
- Typed JavaScript exports use validated `BigInt` for `Int`, `number` for
  `Float`, and explicit decimal strings/objects for `Decimal`. Reject out-of-
  range input instead of inheriting binding-layer integer wrapping.

Do not require Wasm GC, shared memory, threads, or WASI for the initial browser
target. Reconsider Wasm GC only with a measured compatibility/runtime benefit.

### Step 7: Emit Executable Modules From Existing IR

Add `wasm_backend/` over the shared `BackendBundle`, using `wasm-encoder` for
module emission and `wasmparser`/`wasm-tools` for validation. Begin with typed
locals, constants, arithmetic, branches, loops, and direct calls. A basic-block
dispatcher loop is an acceptable first translation of arbitrary IR control
flow; later optimize structured regions.

Then add heap values, constructors, closures, interface calls, collections,
matching/extraction, and cleanup using the ABI from Step 6. Register native
symbols against stdlib descriptors instead of introducing another method-name
switch. Diagnose unsupported features explicitly during incremental delivery.

First compile on the developer machine and run the output in a browser. Only
after that works should browser-side compilation expose `compile(project)`.
Proposed CLI form: `lume build --target browser-wasm`.

Acceptance: generated functions execute Wasm instructions without the IR
interpreter, and work without shipping the compiler to application users.

### Step 8: Prove Parity And Support Embedding

Run shared feature fixtures through native interpretation, browser
interpretation, and compiled Wasm. Cover checked numeric boundaries, decimal
exactness, Unicode, closures, private access, module initialization, equality,
mutable aliasing, argument order, refutable patterns, and failing defers.

Add explicit export/import configuration and generated TypeScript bindings
without changing top-level visibility rules. Specify callback lifetimes,
reentrancy, error translation, handle disposal, and source locations. Browser
HTTP/DOM adapters and async integration are separate APIs; they do not silently
alter existing synchronous signatures.

Acceptance: parity fixtures pass, repeated exported calls have bounded memory,
and an ordinary web application can import a compiled Lume function with no
Lume compiler/interpreter in its shipped execution path.

## Suggested First Implementation Slice

Complete the portability spike, virtual-project pipeline, browser host policy,
and a worker-backed check/run demo first. Add limits before publishing arbitrary
code execution. This delivers practical browser support while validating the
shared foundations before committing to the much larger direct-codegen effort.

## Technical References

- [Rust's wasm32-unknown-unknown target](https://doc.rust-lang.org/stable/rustc/platform-support/wasm32-unknown-unknown.html)
  documents the target's OS-service and panic-strategy limitations.
- [wasm-bindgen numbers](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/numbers.html)
  documents JavaScript `BigInt` mappings and integer conversion behavior.
- [Worker termination](https://developer.mozilla.org/en-US/docs/Web/API/Worker/terminate)
  documents why a hard stop cannot guarantee language cleanup.
- [Bytecode Alliance wasm-tools](https://github.com/bytecodealliance/wasm-tools)
  provides the encoder, parser, and validation tooling for direct emission.
