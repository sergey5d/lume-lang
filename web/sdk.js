const PROTOCOL = 1;
const modules = new Map();

async function compileModule(url) {
  if (!modules.has(url)) {
    const pending = (async () => {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`Could not load Lume Wasm (${response.status}). Run npm run build first.`);
      // Static hosts sometimes omit the Wasm MIME type. Keep a byte fallback.
      if (WebAssembly.compileStreaming) {
        try { return await WebAssembly.compileStreaming(response.clone()); } catch { /* fall back */ }
      }
      return WebAssembly.compile(await response.arrayBuffer());
    })();
    modules.set(url, pending);
    pending.catch(() => modules.delete(url));
  }
  return modules.get(url);
}

export class LumeHostError extends Error {
  constructor(message, code = 'host_error') {
    super(message);
    this.name = 'LumeHostError';
    this.code = code;
  }
}

/** A fresh worker/instance per request prevents globals or reference cycles leaking between runs. */
export class LumeClient {
  #requests = new Set();
  #nextId = 0;
  #disposed = false;

  constructor({ wasmUrl = new URL('./pkg/lume_web_bg.wasm', import.meta.url), workerUrl = new URL('./worker.js', import.meta.url), timeoutMs = 10_000, WorkerClass = globalThis.Worker, moduleLoader = compileModule } = {}) {
    this.wasmUrl = String(wasmUrl);
    this.workerUrl = workerUrl;
    this.timeoutMs = timeoutMs;
    this.WorkerClass = WorkerClass;
    this.moduleLoader = moduleLoader;
  }

  check(project, options) { return this.#request('check', { project }, options); }
  format(source, options = {}) { return this.#request('format', { source, path: options.path ?? 'main.lum' }, options); }
  run(project, options = {}) {
    const { args, environment, entry, limits } = options;
    return this.#request('run', { project, options: { args, environment, entry, limits } }, options);
  }

  #request(operation, payload, { signal, onOutput, timeoutMs = this.timeoutMs } = {}) {
    if (this.#disposed) return Promise.reject(new LumeHostError('Client has been disposed', 'disposed'));
    if (signal?.aborted) return Promise.reject(new LumeHostError('Operation cancelled', 'cancelled'));
    if (!Number.isFinite(timeoutMs) || timeoutMs < 1 || timeoutMs > 30_000) return Promise.reject(new LumeHostError('timeoutMs must be between 1 and 30000', 'invalid_options'));
    return new Promise((resolve, reject) => {
      let worker;
      let timer;
      let settled = false;
      const id = ++this.#nextId;
      const finish = (error, result) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        signal?.removeEventListener('abort', cancel);
        worker?.terminate();
        this.#requests.delete(cancel);
        if (error) reject(error); else resolve(result);
      };
      const cancel = () => finish(new LumeHostError('Operation cancelled; its worker was discarded', 'cancelled'));
      try {
        worker = new this.WorkerClass(this.workerUrl, { type: 'module', name: `lume-${operation}-${id}` });
        this.#requests.add(cancel);
        signal?.addEventListener('abort', cancel, { once: true });
        timer = setTimeout(() => finish(new LumeHostError('Operation timed out; its worker was discarded', 'timeout')), timeoutMs);
        worker.addEventListener('error', event => finish(new LumeHostError(event.message || 'Wasm worker failed')));
        worker.addEventListener('messageerror', () => finish(new LumeHostError('Invalid worker response')));
        worker.addEventListener('message', ({ data }) => {
          if (settled || data?.id !== id) return;
          if (data.protocol !== PROTOCOL) return finish(new LumeHostError('Incompatible worker protocol'));
          if (data.type === 'output') {
            try { onOutput?.(data.text); } catch (error) { finish(error); }
          } else if (data.type === 'result') {
            if (data.result?.protocol !== PROTOCOL) return finish(new LumeHostError('Incompatible Wasm response'));
            finish(null, data.result);
          } else if (data.type === 'fatal') {
            finish(new LumeHostError(data.message, 'fatal'));
          }
        });
        this.moduleLoader(this.wasmUrl).then(module => {
          if (!settled) worker.postMessage({ protocol: PROTOCOL, id, operation, request: JSON.stringify(payload), module });
        }).catch(error => finish(error));
      } catch (error) { finish(error); }
    });
  }

  dispose() {
    this.#disposed = true;
    for (const cancel of [...this.#requests]) cancel();
  }
}

/** Convert Rust UTF-8 byte offsets to textarea/editor UTF-16 offsets. */
export function byteOffsetToUtf16(source, bytes) {
  let byteOffset = 0;
  let utf16Offset = 0;
  for (const scalar of source) {
    const width = scalar.codePointAt(0) <= 0x7f ? 1 : scalar.codePointAt(0) <= 0x7ff ? 2 : scalar.codePointAt(0) <= 0xffff ? 3 : 4;
    if (byteOffset + width > bytes) break;
    byteOffset += width;
    utf16Offset += scalar.length;
  }
  return utf16Offset;
}
