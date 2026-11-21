import initialize, * as lume from './pkg/lume_web.js';

self.onmessage = async ({ data }) => {
  const { id, operation, request, module } = data ?? {};
  const send = payload => self.postMessage({ protocol: 1, id, ...payload });
  let buffered = '';
  let lastFlush = performance.now();
  const flush = () => {
    if (buffered) send({ type: 'output', text: buffered });
    buffered = '';
    lastFlush = performance.now();
  };
  try {
    if (data.protocol !== 1 || !['check', 'run', 'format'].includes(operation) || typeof request !== 'string' || !(module instanceof WebAssembly.Module)) throw new Error('Invalid worker request');
    self.lumeOutput = text => {
      buffered += text;
      if (buffered.length >= 4096 || performance.now() - lastFlush > 50) flush();
    };
    await initialize({ module_or_path: module });
    const result = JSON.parse(lume[operation](request));
    flush();
    send({ type: 'result', result });
  } catch (error) {
    flush();
    send({ type: 'fatal', message: `Lume session failed and was discarded: ${error.message ?? error}` });
  }
};
