import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => { await page.goto('/'); });

test('opening the HTML file directly explains how to serve it', async ({ page }) => {
  const moduleRequests = [];
  page.on('request', request => { if (request.url().endsWith('.js')) moduleRequests.push(request.url()); });
  await page.goto(new URL('../../index.html', import.meta.url).href);
  await expect(page.getByRole('alert')).toContainText('npm --prefix web run serve');
  await expect(page.locator('#status')).toHaveText('HTTP server required');
  for (const name of ['Run', 'Check', 'Format']) {
    await expect(page.getByRole('button', { name, exact: true })).toBeDisabled();
  }
  await expect(page.getByRole('link', { name: 'http://127.0.0.1:8787/' })).toHaveAttribute('href', 'http://127.0.0.1:8787/');
  expect(moduleRequests).toEqual([]);
});

test('a startup module failure is visible instead of leaving inert controls', async ({ page }) => {
  await page.route('**/playground.js', route => route.abort());
  await page.reload();
  await expect(page.locator('#status')).toHaveText('Playground failed to load');
  await expect(page.getByLabel('Program output')).toContainText('Could not load the playground modules');
  await expect(page.getByRole('button', { name: 'Run', exact: true })).toBeDisabled();
});

test('all bundled examples execute in a real Wasm worker', async ({ page }) => {
  for (const [example, expected] of [
    ['hello', 'Total: 10'],
    ['records', 'Total: 20.75'],
    ['modules', 'Price: 20.000'],
    ['json', 'Total: 12'],
    ['limits', 'Cleanup still runs'],
  ]) {
    await page.getByLabel('Choose an example').selectOption(example);
    await page.getByRole('button', { name: 'Run', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeDisabled({ timeout: 30_000 });
    await expect(page.getByLabel('Program output')).toContainText(expected);
    if (example === 'limits') await expect(page.locator('#diagnostics')).toContainText('execution_limit');
    else await expect(page.locator('#status')).toHaveText('Finished');
  }
});

test('SDK preserves numeric precision, host inputs, and collection allocation behavior', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { LumeClient } = await import('/sdk.js');
    const sdk = new LumeClient({ timeoutMs: 30_000 });
    let streamed = '';
    const result = await sdk.run({ entry: 'main.lum', files: { 'main.lum': `fn main() Int {
    rows [[Int]] = Vector.generate(2, index => [])
    rows[0].add(7)
    println(rows[0].size, rows[1].size)
    println(OS.args[0], OS.env["MODE"]!)
    println(0.1d + 0.2d)
    println(9.sqrt(), 1.0.log(), (-7).abs(), "😀é".size)
    9007199254740993
}` } }, { args: ['first'], environment: { MODE: 'browser' }, onOutput: text => { streamed += text; } });
    return { ...result, streamed };
  });
  expect(result.ok, JSON.stringify(result)).toBe(true);
  expect(result.output).toBe('1 0\nfirst browser\n0.3\n3.0 0.0 7 2\n');
  expect(result.streamed).toBe(result.output);
  expect(result.returnValue).toBe('9007199254740993');
});

test('formatting, located diagnostics, and Unicode selection work', async ({ page }) => {
  const editor = page.getByLabel('Source editor');
  await editor.fill('fn main(){println( 1 )}');
  await page.getByRole('button', { name: 'Format', exact: true }).click();
  await expect(page.locator('#status')).toHaveText('Formatted', { timeout: 30_000 });
  await expect(editor).toHaveValue(/println\(1\)/);
  await editor.fill('fn main() {\n    text = "😀"\n    println(missing)\n}');
  await page.getByRole('button', { name: 'Check', exact: true }).click();
  await expect(page.locator('#diagnostics')).toContainText('undefined_name', { timeout: 30_000 });
  await page.locator('.diagnostic').filter({ hasText: 'undefined_name' }).first().click();
  expect(await editor.evaluate(element => element.value.slice(element.selectionStart, element.selectionEnd))).toBe('missing');
});

test('native files are rejected before executing any output', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { LumeClient } = await import('/sdk.js');
    return new LumeClient({ timeoutMs: 30_000 }).run({ entry: 'main.lum', files: { 'main.lum': 'fn main() {\n println("should not run")\n File.readText("secret")\n}' } });
  });
  expect(result.ok).toBe(false);
  expect(result.output).toBe('');
  expect(result.diagnostics.some(error => error.code === 'unsupported_target')).toBe(true);
});

test('imported functions and callbacks retain source ownership in diagnostics', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { LumeClient } = await import('/sdk.js');
    return new LumeClient({ timeoutMs: 30_000 }).run({ entry: 'main.lum', files: {
      'main.lum': 'use helpers/{fail}\nfn main() { fail() }',
      'helpers.lum': 'module helpers\npublic fn fail() { [1].forEach(value => panic("from helper")) }',
    } });
  });
  expect(result.ok).toBe(false);
  expect(result.diagnostics[0].path).toBe('helpers.lum');
  expect(result.diagnostics[0].message).toContain('from helper');
});

test('cancellation and timeout discard a session and allow a fresh run', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { LumeClient } = await import('/sdk.js');
    const sdk = new LumeClient({ timeoutMs: 30_000 });
    const project = { entry: 'main.lum', files: { 'main.lum': 'fn main() { while true {} }' } };
    const controller = new AbortController();
    const cancelled = sdk.run(project, { signal: controller.signal }).catch(error => error.code);
    controller.abort();
    const cancellation = await cancelled;
    const timeout = await sdk.run(project, { timeoutMs: 1 }).catch(error => error.code);
    const recovered = await sdk.run({ entry: 'main.lum', files: { 'main.lum': 'fn main() { println(42) }' } });
    return { cancellation, timeout, recovered };
  });
  expect(result.cancellation).toBe('cancelled');
  expect(result.timeout).toBe('timeout');
  expect(result.recovered.output).toBe('42\n');
});

test('a fatal Wasm stack trap is isolated from subsequent runs', async ({ page }) => {
  const result = await page.evaluate(async () => {
    const { LumeClient } = await import('/sdk.js');
    const sdk = new LumeClient({ timeoutMs: 10_000 });
    const failure = await sdk.run({ entry: 'main.lum', files: { 'main.lum': 'fn main() {\n values [Any] = []\n values.add(values)\n println(values)\n}' } }).catch(error => error.code);
    const recovered = await sdk.run({ entry: 'main.lum', files: { 'main.lum': 'fn main() { println("fresh") }' } });
    return { failure, recovered };
  });
  expect(['fatal', 'timeout']).toContain(result.failure);
  expect(result.recovered.output).toBe('fresh\n');
});

test('the mobile layout fits the viewport', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await expect(page.getByRole('button', { name: 'Run', exact: true })).toBeVisible();
  await expect(page.getByLabel('Source editor')).toBeVisible();
});
