import { test } from 'node:test';
import assert from 'node:assert/strict';
import { LumeClient, byteOffsetToUtf16 } from '../sdk.js';

const project = { entry: 'main.lum', files: { 'main.lum': 'fn main() {}' } };
const result = { protocol: 1, ok: true, diagnostics: [], warnings: [], output: 'hello\n', returnValue: null };

class FakeWorker extends EventTarget {
  static instances = [];
  static respond;
  constructor() { super(); FakeWorker.instances.push(this); }
  postMessage(request) { this.request = request; FakeWorker.respond?.(this, request); }
  message(data) { this.dispatchEvent(new MessageEvent('message', { data: { protocol: 1, id: this.request.id, ...data } })); }
  terminate() { this.terminated = true; }
}

function client(respond, moduleLoader = async () => ({})) {
  FakeWorker.instances = [];
  FakeWorker.respond = respond;
  return new LumeClient({ WorkerClass: FakeWorker, moduleLoader, timeoutMs: 100 });
}

test('streams output, returns structured results, and disposes each worker', async () => {
  const sdk = client(worker => {
    worker.message({ type: 'output', text: 'hello\n' });
    worker.message({ type: 'result', result });
  });
  let output = '';
  assert.deepEqual(await sdk.run(project, { onOutput: text => { output += text; } }), result);
  assert.equal(output, 'hello\n');
  await sdk.check(project);
  assert.equal(FakeWorker.instances.length, 2);
  assert.ok(FakeWorker.instances.every(worker => worker.terminated));
});

test('abort discards the worker even while the module is loading', async () => {
  const sdk = client(null, () => new Promise(() => {}));
  const controller = new AbortController();
  const pending = sdk.run(project, { signal: controller.signal });
  controller.abort();
  await assert.rejects(pending, error => error.code === 'cancelled');
  assert.ok(FakeWorker.instances[0].terminated);
  assert.equal(FakeWorker.instances[0].request, undefined);
});

test('a main-thread watchdog stops unresponsive work', async () => {
  const sdk = client();
  await assert.rejects(sdk.run(project, { timeoutMs: 5 }), error => error.code === 'timeout');
  assert.ok(FakeWorker.instances[0].terminated);
});

test('fatal Wasm failures do not poison subsequent requests', async () => {
  const sdk = client(worker => worker.message({ type: 'fatal', message: 'unreachable' }));
  await assert.rejects(sdk.run(project), error => error.code === 'fatal');
  FakeWorker.respond = worker => worker.message({ type: 'result', result });
  assert.equal((await sdk.run(project)).ok, true);
  assert.ok(FakeWorker.instances.every(worker => worker.terminated));
});

test('disposal cancels pending requests and rejects new ones', async () => {
  const sdk = client();
  const pending = sdk.check(project);
  sdk.dispose();
  await assert.rejects(pending, error => error.code === 'cancelled');
  await assert.rejects(sdk.check(project), error => error.code === 'disposed');
});

test('module and protocol failures dispose their workers', async () => {
  const sdk = client(null, async () => { throw new Error('download failed'); });
  await assert.rejects(sdk.check(project), /download failed/);
  assert.ok(FakeWorker.instances[0].terminated);
  const bad = client(worker => worker.message({ type: 'result', result: { protocol: 42 } }));
  await assert.rejects(bad.check(project), /Incompatible Wasm response/);
  assert.ok(FakeWorker.instances[0].terminated);
});

test('format sends only its source and path, without client callback options', async () => {
  const sdk = client((worker, request) => {
    assert.equal(request.operation, 'format');
    assert.deepEqual(JSON.parse(request.request), { source: 'fn main() {}', path: 'other.lum' });
    worker.message({ type: 'result', result: { ...result, text: 'fn main() {}\n' } });
  });
  assert.equal((await sdk.format('fn main() {}', { path: 'other.lum' })).text, 'fn main() {}\n');
});

test('UTF-8 byte spans convert to UTF-16 without splitting surrogate pairs', () => {
  const text = 'a😀éz';
  assert.equal(byteOffsetToUtf16(text, 1), 1);
  assert.equal(byteOffsetToUtf16(text, 5), 3);
  assert.equal(byteOffsetToUtf16(text, 7), 4);
  assert.equal(byteOffsetToUtf16(text, 999), text.length);
  assert.equal(byteOffsetToUtf16(text, 3), 1);
});

test('invalid deadlines and pre-aborted requests never create a worker', async () => {
  const sdk = client();
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(sdk.check(project, { signal: controller.signal }), error => error.code === 'cancelled');
  await assert.rejects(sdk.check(project, { timeoutMs: 0.5 }), error => error.code === 'invalid_options');
  assert.equal(FakeWorker.instances.length, 0);
});
