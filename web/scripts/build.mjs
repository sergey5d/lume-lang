import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../../', import.meta.url));
function execute(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

// Use the same toolchain that rustup installed the Wasm standard library for.
const toolchain = process.env.LUME_RUST_TOOLCHAIN ?? 'stable';
const compiler = spawnSync('rustup', ['which', '--toolchain', toolchain, 'rustc'], { encoding: 'utf8' });
if (compiler.status !== 0) throw new Error('A rustup toolchain is required to build the browser package.');
execute('rustup', ['run', toolchain, 'cargo', 'build', '--manifest-path', 'rust/Cargo.toml', '-p', 'lume-web', '--target', 'wasm32-unknown-unknown', '--release'], {
  env: { ...process.env, RUSTC: compiler.stdout.trim(), RUSTFLAGS: `${process.env.RUSTFLAGS ?? ''} -C link-arg=--max-memory=268435456` },
});
const lock = readFileSync(path.join(root, 'rust/Cargo.lock'), 'utf8');
const version = lock.match(/name = "wasm-bindgen"\nversion = "([^"]+)"/)[1];
const bindgen = process.env.WASM_BINDGEN ?? 'wasm-bindgen';
const installed = spawnSync(bindgen, ['--version'], { encoding: 'utf8' });
if (installed.error || installed.stdout.trim() !== `wasm-bindgen ${version}`) {
  throw new Error(`Install the matching tool: cargo install wasm-bindgen-cli --version ${version} --locked\nOr set WASM_BINDGEN to its executable path.`);
}
const target = process.env.CARGO_TARGET_DIR ? path.resolve(root, process.env.CARGO_TARGET_DIR) : path.join(root, 'rust/target');
execute(bindgen, [path.join(target, 'wasm32-unknown-unknown/release/lume_web.wasm'), '--target', 'web', '--out-dir', path.join(root, 'web/pkg'), '--typescript']);
console.log('Browser package built. Run: cd web && npm run serve');
