# Lume In The Browser

This package compiles the existing Rust compiler/interpreter to WebAssembly.
Lume programs are checked and interpreted locally inside disposable Web Workers;
this is not a direct Lume-to-Wasm code-generation backend.

## Build And Run

Requires Node.js 20+, a stable rustup toolchain, and the matching wasm-bindgen CLI.

```sh
rustup target add wasm32-unknown-unknown --toolchain stable
```

Find the `wasm-bindgen` version in `rust/Cargo.lock`, then install that exact CLI
version. Currently:

```sh
cargo install wasm-bindgen-cli --version 0.2.130 --locked
cd web
npm ci
npm run build
npm run serve
```

Open `http://127.0.0.1:8787`, not `web/index.html` directly. Browsers block the
module scripts and workers when the page uses a `file://` URL. If the Wasm assets
are already built, only `npm --prefix web run serve` from the repository root is
needed to start the playground again.

The build uses rustup's compiler explicitly, even
when a Homebrew Rust installation appears first on `PATH`. Set
`LUME_RUST_TOOLCHAIN` or `WASM_BINDGEN` to override the toolchain or CLI executable.
Generated assets live in `web/pkg/` and are not committed.

The playground uses a minimal CodeMirror 6 editor with highlighting, line numbers,
bracket matching, and per-file undo history. The editor is bundled locally; it
does not load scripts from a CDN. Its keyword list is generated from the Rust
lexer during the build. `npm run build:editor` rebuilds only the editor without
recompiling Wasm; `npm run serve` also builds it before starting the server.

For deployment, publish this directory's static `.html`, `.css`, `.js`, and
`pkg/` assets. No application server, JVM, shared memory, or cross-origin isolation
headers are required. Serve `.wasm` as `application/wasm` when possible. If your
host sets CSP, allow the same-origin worker and WebAssembly compilation (for
example `worker-src 'self'` and the appropriate `wasm-unsafe-eval` script policy).
Do not open the playground with a `file://` URL.

## JavaScript / TypeScript SDK

```js
import { LumeClient } from './sdk.js';

const lume = new LumeClient();
const project = {
  entry: 'main.lum',
  files: { 'main.lum': 'fn main() { println("hello") }' },
};

const checked = await lume.check(project);
const formatted = await lume.format(project.files['main.lum']);
const result = await lume.run(project, {
  args: ['--verbose'],
  environment: { MODE: 'demo' },
  onOutput: text => console.log(text),
});

console.log(result.output, result.returnValue, result.diagnostics);
lume.dispose();
```

Use `AbortController.signal` to stop a request. `timeoutMs` defaults to 10 seconds
and may be set up to 30 seconds. Normal compile/runtime errors resolve to
structured diagnostics. Cancellation, timeouts, worker failures, and fatal Wasm
traps reject with `LumeHostError`; a later request uses a new worker.

`sdk.d.ts` describes the public API. Diagnostic offsets are UTF-8 bytes; use
`byteOffsetToUtf16` before selecting text in a JavaScript editor. Return values
are display strings, so `Int` and `Decimal` never lose precision through JS
`number` or JSON numeric conversion. Arbitrary Lume values are not exported as
live JavaScript objects.

## Browser Contract

- Projects contain at most 64 files and 1 MiB total UTF-8 source. Paths are
  relative; imports resolve only supplied files and embedded JSON/regex modules.
- `OS.args` and `OS.env` use explicitly supplied values, empty by default.
- Native file operations are rejected before execution, with runtime guards as
  defense in depth. There is no Java/JAR loading, native process access, network
  import resolver, implicit DOM access, or filesystem fallback.
- Runs default to 1,000,000 execution steps, call depth 128, 100,000 collection
  entries, 1 MiB strings, and 64 KiB output. SDK limit options may only reduce
  these bounds. Cleanup after a managed limit error has a separate 10,000-step
  allowance. Expensive operations also face the worker's wall-clock deadline.
- The generated module has a 256 MiB maximum linear memory. Allocation failure,
  Rust panics, stack overflow, and uncooperative native work may cause fatal
  termination rather than a recoverable diagnostic.
- Stop/timeout immediately discards the worker; it cannot guarantee `defer`.
  Ordinary language-managed errors still run bounded cleanup. Every request
  gets a fresh Wasm instance, reclaiming reference cycles and global state.
- This is an execution boundary, not a claim of protection against every
  browser resource-exhaustion attack. Keep imports minimal when embedding it.

## Tests

```sh
cargo test --manifest-path ../rust/Cargo.toml -p lume --lib project::tests
cargo test --manifest-path ../rust/Cargo.toml -p lume-web
npm test
npm install
npx playwright install chromium firefox webkit
npm run test:browser
```

Build the Wasm package before browser tests. The browser suite covers bundled
examples, multi-file imports, JSON/regex, exact numbers, Unicode diagnostics,
formatting, host rejection, cancellation, traps/recovery, and mobile layout.
